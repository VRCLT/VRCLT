use super::{
    menu::Command,
    placement::{Panel, Placement, Pose, Resize, Size},
    wrist,
};

#[derive(Clone, Copy)]
pub struct Frame {
    pub head: Pose,
    pub wrist: Option<Pose>,
    pub pointer: Pose,
    pub trigger: bool,
    pub grip: bool,
}
#[derive(Default)]
pub struct Feedback {
    pub click: Option<Command>,
    pub wrist_uv: Option<(f32, f32)>,
    pub placed: Option<Placement>,
    pub beam: Option<(Pose, f32)>,
    pub cursor: Option<Pose>,
}
#[derive(Clone, Copy)]
enum Drag {
    Move { original: Placement, offset: Pose },
    Resize { original: Placement, resize: Resize },
}
impl Drag {
    fn original(self) -> Placement {
        match self {
            Self::Move { original, .. } | Self::Resize { original, .. } => original,
        }
    }
}
pub struct Hands {
    pub watch: Pose,
    pub captions: Pose,
    pub size: Size,
    pub engaged: bool,
    drag: Option<Drag>,
    pressed: Option<Command>,
    trigger: bool,
    grip: bool,
}
impl Hands {
    pub fn new(watch: Pose, captions: Pose, size: Size) -> Self {
        Self {
            watch,
            captions,
            size,
            engaged: false,
            drag: None,
            pressed: None,
            trigger: true,
            grip: true,
        }
    }
    fn placement(&self, panel: Panel) -> Placement {
        Placement {
            panel,
            pose: if panel == Panel::Wrist {
                self.watch
            } else {
                self.captions
            },
            size: (panel == Panel::Captions).then_some(self.size),
        }
    }
    pub fn apply(&mut self, p: Placement) {
        match p.panel {
            Panel::Wrist => self.watch = p.pose,
            Panel::Captions => {
                self.captions = p.pose;
                if let Some(s) = p.size {
                    self.size = s;
                }
            }
        }
    }
    pub fn cancel(&mut self) {
        if let Some(d) = self.drag.take() {
            self.apply(d.original());
        }
        self.pressed = None;
        self.trigger = true;
        self.grip = true;
        self.engaged = false;
    }
    pub fn pressed(&self) -> Option<&Command> {
        self.pressed.as_ref()
    }
    pub fn tick(
        &mut self,
        input: Option<Frame>,
        edit: Option<Panel>,
        layout: &wrist::Layout,
        wrist_width: f32,
    ) -> Feedback {
        let mut out = Feedback::default();
        let Some(frame) = input else {
            self.cancel();
            return out;
        };
        let wrist = frame.wrist.map(|w| w.compose(self.watch));
        if let Some(d) = self.drag {
            if edit != Some(d.original().panel)
                || (d.original().panel == Panel::Wrist && wrist.is_none())
            {
                self.cancel();
            }
        }
        let was_engaged = self.engaged;
        self.engaged = wrist.is_some_and(|w| frame.head.looking_at(w, self.engaged))
            || self
                .drag
                .is_some_and(|d| d.original().panel == Panel::Wrist);
        let caption_edit = edit == Some(Panel::Captions);
        let watch_size = Size {
            width: wrist_width,
            height: wrist_width * wrist::HEIGHT as f32 / wrist::WIDTH as f32,
        };
        let watch_hit = wrist
            .filter(|_| self.engaged)
            .and_then(|w| w.hit(frame.pointer, 1.2))
            .filter(|&p| watch_size.contains(p, 0.));
        let captions = frame.head.compose(self.captions);
        let caption_hit = caption_edit
            .then(|| captions.hit(frame.pointer, 2.))
            .flatten()
            .filter(|&p| self.size.contains(p, 0.04));
        let target = watch_hit
            .map(|xy| (Panel::Wrist, wrist.unwrap(), xy))
            .or_else(|| caption_hit.map(|xy| (Panel::Captions, captions, xy)));
        if let Some((_, base, [x, y])) = target {
            out.cursor = Some(base.offset([x, y, 0.004]));
        }
        if self.engaged || caption_edit {
            let length = out
                .cursor
                .map(|p| distance(frame.pointer.pos(), p.pos()))
                .unwrap_or(if caption_edit { 1.4 } else { 0.512 })
                .clamp(0.03, 2.5);
            out.beam = Some((beam_pose(frame.pointer, frame.head, length), length));
        }
        let wrist_uv =
            watch_hit.map(|[x, y]| (0.5 + x / watch_size.width, 0.5 + y / watch_size.height));
        let click = wrist_uv.and_then(|(u, v)| layout.hit(u, v));
        let fresh_trigger = frame.trigger && !self.trigger;
        let fresh_grip = frame.grip && !self.grip;
        if self.drag.is_none() {
            if let Some((panel, world, xy)) = target.filter(|(p, _, _)| Some(*p) == edit) {
                let original = self.placement(panel);
                if panel == Panel::Captions && fresh_trigger {
                    self.drag = self.size.corner(xy).map(|side| Drag::Resize {
                        original,
                        resize: Resize::new(self.captions, self.size, side, xy),
                    });
                }
                if self.drag.is_none() && fresh_grip {
                    self.drag = Some(Drag::Move {
                        original,
                        offset: frame.pointer.inverse().compose(world),
                    });
                }
            }
        }
        match self.drag {
            Some(d) => {
                self.pressed = None;
                let original = d.original();
                let held = match d {
                    Drag::Move { offset, .. } if frame.grip => {
                        let anchor = if original.panel == Panel::Wrist {
                            frame.wrist.unwrap()
                        } else {
                            frame.head
                        };
                        self.apply(Placement {
                            pose: anchor.inverse().compose(frame.pointer).compose(offset),
                            ..original
                        });
                        true
                    }
                    Drag::Resize { resize, .. } if frame.trigger => {
                        if let Some((pose, size)) =
                            resize.update(frame.head.inverse().compose(frame.pointer))
                        {
                            self.captions = pose;
                            self.size = size;
                        }
                        true
                    }
                    _ => false,
                };
                if !held {
                    out.placed = Some(self.placement(original.panel));
                    self.drag = None;
                }
            }
            None => {
                out.wrist_uv = wrist_uv;
                if !self.engaged {
                    self.pressed = None;
                }
                if fresh_trigger && was_engaged {
                    self.pressed = click.cloned();
                }
                if !frame.trigger && self.trigger {
                    out.click = self.pressed.take().filter(|cmd| Some(cmd) == click);
                }
            }
        }
        self.trigger = frame.trigger;
        self.grip = frame.grip;
        out
    }
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
// Face the beam's narrow strip toward the viewer even if the controller rolls.
fn beam_pose(pointer: Pose, head: Pose, length: f32) -> Pose {
    let center = pointer.offset([0., 0., -length / 2.]).pos();
    let y = pointer.0.map(|r| r[2]);
    let side = cross(y, std::array::from_fn(|i| head.0[i][3] - center[i]));
    let norm = distance(side, [0.; 3]);
    let x = if norm > 1e-5 {
        side.map(|v| v / norm)
    } else {
        pointer.0.map(|r| r[0])
    };
    let z = cross(x, y);
    Pose(std::array::from_fn(|r| [x[r], y[r], z[r], center[r]]))
}
