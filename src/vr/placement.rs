use crate::config::Config;
use std::array::from_fn;

/// OpenVR row-major rigid transform, in metres relative to its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose(pub [[f32; 4]; 3]);

impl Pose {
    pub const IDENTITY: Self = Self([[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]]);

    pub fn at(mut self, xyz: [f32; 3]) -> Self {
        for (row, x) in self.0.iter_mut().zip(xyz) {
            row[3] = x;
        }
        self
    }
    pub fn pos(self) -> [f32; 3] {
        self.0.map(|r| r[3])
    }
    pub fn pitch(deg: f32) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        Self([[1., 0., 0., 0.], [0., c, -s, 0.], [0., s, c, 0.]])
    }
    pub fn compose(self, child: Self) -> Self {
        Self(from_fn(|r| {
            from_fn(|c| {
                let base = if c == 3 { self.0[r][3] } else { 0. };
                base + (0..3).map(|i| self.0[r][i] * child.0[i][c]).sum::<f32>()
            })
        }))
    }
    pub fn inverse(self) -> Self {
        Self(from_fn(|r| {
            from_fn(|c| {
                if c == 3 {
                    -(0..3).map(|i| self.0[i][r] * self.0[i][3]).sum::<f32>()
                } else {
                    self.0[c][r]
                }
            })
        }))
    }
    pub fn offset(self, xyz: [f32; 3]) -> Self {
        self.compose(Self::IDENTITY.at(xyz))
    }

    pub fn looking_at(self, center: Self, engaged: bool) -> bool {
        let [x, y, z] = self.inverse().compose(center).pos();
        let distance = (x * x + y * y + z * z).sqrt();
        let (angle, limit) = if engaged {
            (35_f32, 0.95 * 1.3)
        } else {
            (22_f32, 0.95)
        };
        distance > 0.0001 && distance < limit && -z > distance * angle.to_radians().cos()
    }
    // Intersect the controller's -Z ray with this panel's (unbounded) XY plane.
    pub fn hit(self, pointer: Self, limit: f32) -> Option<[f32; 2]> {
        let local = self.inverse().compose(pointer);
        let [x, y, z] = local.pos();
        let dz = local.0[2][2];
        if dz.abs() < 1e-5 {
            return None;
        }
        let t = z / dz;
        (0. ..=limit)
            .contains(&t)
            .then(|| [x - t * local.0[0][2], y - t * local.0[1][2]])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Wrist,
    Captions,
}
impl Panel {
    pub fn section(self) -> &'static str {
        match self {
            Self::Wrist => "wrist_ui",
            Self::Captions => "overlay",
        }
    }
    pub fn configured(self, cfg: &Config) -> Pose {
        let section = self.section();
        let read = |v: &serde_json::Value| {
            let rows = v.as_array()?.get(..3)?;
            let row = |i: usize| serde_json::from_value::<[f32; 4]>(rows[i].clone()).ok();
            Some([row(0)?, row(1)?, row(2)?])
                .filter(|m| m.iter().flatten().all(|n| n.is_finite()))
                .map(Pose)
        };
        if let Some(pose) = read(&cfg.data[section]["transform"]) {
            return pose;
        }
        let filename = if self == Self::Wrist {
            "wrist_transform.json"
        } else {
            "subtitle_transform.json"
        };
        if let Some(pose) = std::fs::read(Config::path().with_file_name(filename))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .and_then(|v| read(&v))
        {
            return pose;
        }
        self.fallback(cfg)
    }
    pub fn fallback(self, cfg: &Config) -> Pose {
        match self {
            Self::Captions => Pose::pitch(cfg.n("/overlay/tilt_deg", -15.) as f32).at([
                0.,
                -cfg.n("/overlay/below_m", 0.35) as f32,
                -cfg.n("/overlay/distance_m", 1.2) as f32,
            ]),
            Self::Wrist => {
                let (s, c) = (cfg.n("/wrist_ui/roll_deg", 0.) as f32)
                    .to_radians()
                    .sin_cos();
                let roll = Pose([[c, -s, 0., 0.], [s, c, 0., 0.], [0., 0., 1., 0.]]);
                let offset = serde_json::from_value(cfg.data[self.section()]["offset"].clone())
                    .unwrap_or([0., 0.02, 0.12]);
                Pose::pitch(cfg.n("/wrist_ui/tilt_deg", 0.) as f32 - 90.)
                    .compose(roll)
                    .at(offset)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}
impl Size {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: width.clamp(0.45, 1.6),
            height: height.clamp(0.10, 0.60),
        }
    }
    pub fn configured(cfg: &Config) -> Self {
        Self::new(
            cfg.n("/overlay/width_m", 0.9) as f32,
            cfg.n("/overlay/height_m", 0.225) as f32,
        )
    }
    pub fn contains(self, [x, y]: [f32; 2], margin: f32) -> bool {
        x.abs() <= self.width / 2. + margin && y.abs() <= self.height / 2. + margin
    }
    pub fn corner(self, [x, y]: [f32; 2]) -> Option<f32> {
        if self.contains([x, y], 0.04)
            && y <= -self.height / 2. + (self.height * 0.275).max(0.055)
            && x.abs() >= self.width / 2. - (self.width * 0.125).max(0.12)
        {
            Some(x.signum())
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub panel: Panel,
    pub pose: Pose,
    pub size: Option<Size>,
}
impl Placement {
    pub fn save_to(self, cfg: &mut Config) {
        let section = &mut cfg.data[self.panel.section()];
        section["transform"] = serde_json::json!(self.pose.0);
        if let Some(size) = self.size {
            section["width_m"] = size.width.into();
            section["height_m"] = size.height.into();
        }
    }
}

#[derive(Clone, Copy)]
pub struct Resize {
    base: Pose,
    size: Size,
    side: f32,
    start: [f32; 2],
}
impl Resize {
    pub fn new(base: Pose, size: Size, side: f32, start: [f32; 2]) -> Self {
        Self {
            base,
            size,
            side,
            start,
        }
    }
    pub fn update(self, pointer_in_head: Pose) -> Option<(Pose, Size)> {
        let [x, y] = self.base.hit(pointer_in_head, 2.5)?;
        let size = Size::new(
            self.size.width + self.side * (x - self.start[0]),
            self.size.height - (y - self.start[1]),
        );
        let pose = self.base.offset([
            self.side * (size.width - self.size.width) / 2.,
            -(size.height - self.size.height) / 2.,
            0.,
        ]);
        Some((pose, size))
    }
}
