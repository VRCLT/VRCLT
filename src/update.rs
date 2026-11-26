use crate::platform::Handle;
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Url, blocking::Client, redirect::Policy};
use semver::Version;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
    process::{ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;
use windows_sys::Win32::{
    Foundation::{INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    System::{Diagnostics::ToolHelp::*, Threading::*},
};

const LATEST: &str = "https://api.github.com/repos/VRCLT/VRCLT/releases/latest";
const DOWNLOAD: &str = "https://github.com/VRCLT/VRCLT/releases/download";
const PERIOD: Duration = Duration::from_secs(6 * 3600);

struct Release {
    version: Version,
    url: String,
    size: u64,
    digest: String,
}

fn release(data: &Value, current: &Version) -> Result<Option<Release>> {
    ensure!(
        data["draft"] == false && data["prerelease"] == false,
        "Expected a published stable release"
    );
    let tag = data["tag_name"].as_str().context("Missing release tag")?;
    let version = Version::parse(tag.strip_prefix('v').context("Invalid release tag")?)?;
    if !version.pre.is_empty() || version.cmp_precedence(current).is_le() {
        return Ok(None);
    }
    let name = format!("vrclt-{tag}-windows-x64.exe");
    let asset = data["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|v| v["name"] == name))
        .context("The Windows download is unavailable")?;
    let url = format!("{DOWNLOAD}/{tag}/{name}");
    ensure!(
        asset["browser_download_url"] == url && asset["state"] == "uploaded",
        "Invalid Windows download"
    );
    let digest = asset["digest"]
        .as_str()
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .context("Missing SHA-256 digest")?;
    let size = asset["size"]
        .as_u64()
        .filter(|s| (1..=512 * 1024 * 1024).contains(s))
        .context("Invalid download size")?;
    Ok(Some(Release {
        version,
        url,
        size,
        digest: digest.to_ascii_lowercase(),
    }))
}

fn trusted_redirect(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "api.github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}

fn product_version(path: &Path) -> Result<Version> {
    use windows_sys::Win32::Storage::FileSystem::*;
    let name: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let size = GetFileVersionInfoSizeW(name.as_ptr(), std::ptr::null_mut());
        ensure!(size > 0, "Missing VRCLT version information");
        let mut block = vec![0u32; (size as usize).div_ceil(4)];
        ensure!(
            GetFileVersionInfoW(name.as_ptr(), 0, size, block.as_mut_ptr().cast()) != 0,
            "Cannot read version information"
        );
        // build.rs embeds language-neutral Unicode product resources in each packaged EXE.
        let field = |key: &str| -> Result<String> {
            let query = crate::platform::wide(&format!(r"\StringFileInfo\000004b0\{key}"));
            let mut ptr = std::ptr::null_mut();
            let mut len = 0;
            ensure!(
                VerQueryValueW(block.as_ptr().cast(), query.as_ptr(), &mut ptr, &mut len) != 0
                    && !ptr.is_null()
                    && len > 0,
                "Missing product field {key}"
            );
            Ok(String::from_utf16(std::slice::from_raw_parts(
                ptr.cast(),
                (len - 1) as usize,
            ))?)
        };
        ensure!(
            field("ProductName")? == "VRCLT",
            "Expected a VRCLT executable"
        );
        Ok(Version::parse(&field("ProductVersion")?)?)
    }
}

fn cache() -> Result<PathBuf> {
    Ok(
        PathBuf::from(env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable")?)
            .join("vrclt/updates"),
    )
}

fn clean_downloads(dir: &Path) {
    if let Ok(files) = fs::read_dir(dir) {
        for file in files.flatten() {
            let name = file.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("vrclt-stage-") && name.ends_with(".exe") {
                let _ = fs::remove_file(file.path());
            }
        }
    }
}

fn verified_copy(
    mut source: impl Read,
    mut out: impl Write,
    size: u64,
    digest: &str,
    mut progress: impl FnMut(u64) -> Result<()>,
) -> Result<()> {
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut buf = vec![0; 128 * 1024];
    loop {
        let n = source.read(&mut buf)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        ensure!(bytes <= size, "The download exceeds the published size");
        out.write_all(&buf[..n])?;
        hash.update(&buf[..n]);
        progress(bytes)?;
    }
    ensure!(
        bytes == size && format!("{:x}", hash.finalize()) == digest,
        "The download failed SHA-256 verification"
    );
    Ok(())
}

pub enum Status {
    Idle,
    Checking,
    Downloading(u8),
    Current,
    Ready(Version, ChildStdin),
    Error(String),
}

pub struct Updater {
    pub status: Status,
    job: Option<Receiver<Status>>,
    next: Instant,
}

impl Default for Updater {
    fn default() -> Self {
        let failure = cache()
            .ok()
            .map(|p| p.join("install-error.txt"))
            .and_then(|p| {
                let message = fs::read_to_string(&p).ok()?;
                let _ = fs::remove_file(p);
                Some(message)
            });
        let next = Instant::now()
            + if failure.is_some() {
                PERIOD
            } else {
                Duration::ZERO
            };
        Self {
            status: failure.map_or(Status::Idle, Status::Error),
            job: None,
            next,
        }
    }
}

impl Updater {
    pub fn check(&mut self, ctx: &eframe::egui::Context) {
        if self.job.is_some() || matches!(self.status, Status::Ready(..)) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        self.status = Status::Checking;
        let ctx = ctx.clone();
        thread::spawn(move || {
            let send = |status| -> Result<()> {
                tx.send(status).map_err(|_| anyhow::anyhow!("Cancelled"))?;
                crate::platform::wake_console(&ctx);
                Ok(())
            };
            let result = check_release(&send);
            let _ = send(result.unwrap_or_else(|e| Status::Error(format!("{e:#}"))));
        });
    }

    pub fn tick(&mut self, auto: bool, ctx: &eframe::egui::Context) {
        if let Some(rx) = &self.job {
            while let Ok(status) = rx.try_recv() {
                self.status = status;
            }
            if !matches!(self.status, Status::Checking | Status::Downloading(_)) {
                self.job = None;
                self.next = Instant::now() + PERIOD;
            }
        }
        if auto && self.job.is_none() && !matches!(self.status, Status::Ready(..)) {
            let wait = self.next.saturating_duration_since(Instant::now());
            if wait.is_zero() {
                self.check(ctx);
            } else {
                ctx.request_repaint_after(wait);
            }
        }
    }

    pub fn cancel(&mut self) {
        self.job = None;
        self.status = Status::Idle;
        self.next = Instant::now();
    }

    pub fn install(&mut self, restart: bool) -> bool {
        let Status::Ready(_, pipe) = &mut self.status else {
            return false;
        };
        match writeln!(pipe, "{}", if restart { "restart" } else { "install" }) {
            Ok(()) => {
                self.status = Status::Idle;
                true
            }
            Err(e) => {
                self.status = Status::Error(e.to_string());
                false
            }
        }
    }
}

fn check_release(send: impl Fn(Status) -> Result<()>) -> Result<Status> {
    let client = Client::builder()
        .user_agent(concat!("VRCLT/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(12))
        .redirect(Policy::custom(|r| {
            if r.previous().len() < 5 && trusted_redirect(r.url()) {
                r.follow()
            } else {
                r.error("Untrusted update redirect")
            }
        }))
        .build()?;
    let dir = cache()?;
    clean_downloads(&dir);
    let data = client
        .get(LATEST)
        .timeout(Duration::from_secs(25))
        .send()?
        .error_for_status()?
        .json()?;
    let Some(update) = release(&data, &Version::parse(env!("CARGO_PKG_VERSION"))?)? else {
        return Ok(Status::Current);
    };
    send(Status::Downloading(0))?;
    fs::create_dir_all(&dir)?;
    let mut stage = tempfile::Builder::new()
        .prefix("vrclt-stage-")
        .suffix(".exe")
        .tempfile_in(dir)?;
    let body = client
        .get(&update.url)
        .timeout(Duration::from_secs(600))
        .send()?
        .error_for_status()?;
    let mut last = 0;
    verified_copy(
        body,
        stage.as_file_mut(),
        update.size,
        &update.digest,
        |bytes| {
            let percent = (bytes * 100 / update.size) as u8;
            if percent != last {
                send(Status::Downloading(percent))?;
                last = percent;
            }
            Ok(())
        },
    )?;
    stage.as_file().sync_all()?;
    let path = stage.into_temp_path();
    // Hold a read-only sharing lock between verification and CreateProcess.
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
        .open(&path)?;
    verified_copy(&locked, io::sink(), update.size, &update.digest, |_| Ok(()))?;
    ensure!(
        product_version(&path)? == update.version,
        "The executable version does not match the release"
    );
    let mut child = Command::new(&path)
        .arg("--finish-update")
        .arg(&update.digest)
        .args(env::args_os().skip(1))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("Could not start the verified update")?;
    drop(locked);
    let mut reply = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut reply)?;
    ensure!(
        reply.trim_end() == "READY",
        "{}",
        reply
            .trim()
            .strip_prefix("ERROR ")
            .unwrap_or("Could not prepare the update")
    );
    let pipe = child.stdin.take().unwrap();
    thread::spawn(move || {
        let _ = child.wait();
        drop(path);
    });
    Ok(Status::Ready(update.version, pipe))
}

// Resolve the installing app from the actual parent process, never from a caller-provided path.
fn parent_image() -> Result<(Handle, PathBuf)> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error().into());
        }
        let snapshot = Handle(snapshot);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        let mut more = Process32FirstW(snapshot.0, &mut entry);
        while more != 0 && entry.th32ProcessID != std::process::id() {
            more = Process32NextW(snapshot.0, &mut entry);
        }
        ensure!(more != 0, "Cannot locate the running VRCLT process");
        let proc = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            entry.th32ParentProcessID,
        );
        if proc.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let proc = Handle(proc);
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        if QueryFullProcessImageNameW(proc.0, 0, buf.as_mut_ptr(), &mut len) == 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok((
            proc,
            std::ffi::OsString::from_wide(&buf[..len as usize]).into(),
        ))
    }
}

pub fn finish_install(args: &[String]) -> Result<()> {
    let hash = args.first().context("Missing update digest")?;
    let (parent, target) = parent_image()?;
    let new_version = Version::parse(env!("CARGO_PKG_VERSION"))?;
    ensure!(
        new_version
            .cmp_precedence(&product_version(&target)?)
            .is_gt(),
        "The installed VRCLT version is already current"
    );
    let dir = target.parent().context("Missing installation folder")?;
    let source = fs::File::open(env::current_exe()?)?;
    let size = source.metadata()?.len();
    let mut next =
        NamedTempFile::new_in(dir).context("Cannot write to the VRCLT installation folder")?;
    verified_copy(source, next.as_file_mut(), size, hash, |_| Ok(()))?;
    next.as_file().sync_all()?;
    writeln!(io::stdout().lock(), "READY")?;
    io::stdout().flush()?;
    let mut command = String::new();
    io::stdin().lock().read_line(&mut command)?;
    let restart = match command.trim_start_matches('\u{feff}').trim_end() {
        "install" => false,
        "restart" => true,
        _ => return Ok(()), // App crashed, or the user disabled automatic updates.
    };
    let apply = || -> Result<()> {
        ensure!(
            unsafe { WaitForSingleObject(parent.0, 60_000) } == WAIT_OBJECT_0,
            "Timed out waiting for VRCLT to close"
        );
        ensure!(
            new_version
                .cmp_precedence(&product_version(&target)?)
                .is_gt(),
            "Another update already replaced VRCLT"
        );
        // persist performs a single same-volume replacement. Keep the installed file on failure.
        let mut pending = next.into_temp_path();
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
            .open(&pending)?;
        verified_copy(&locked, io::sink(), size, hash, |_| Ok(()))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match pending.persist(&target) {
                Ok(()) => break,
                Err(e)
                    if Instant::now() < deadline
                        && matches!(e.error.raw_os_error(), Some(5 | 32 | 33)) =>
                {
                    pending = e.path;
                    thread::sleep(Duration::from_millis(150));
                }
                Err(e) => bail!("Could not install the update: {}", e.error),
            }
        }
        Ok(())
    };
    let result = apply();
    if let Err(e) = &result {
        let _ = fs::write(cache()?.join("install-error.txt"), format!("{e:#}"));
    }
    if restart {
        Command::new(target)
            .args(&args[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Could not reopen VRCLT")?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest(tag: &str) -> Value {
        let name = format!("vrclt-{tag}-windows-x64.exe");
        json!({"draft": false, "prerelease": false, "tag_name": tag, "assets": [{
            "name": name, "state": "uploaded", "size": 123, "digest": format!("sha256:{}", "0".repeat(64)),
            "browser_download_url": format!("{DOWNLOAD}/{tag}/{name}")
        }]})
    }

    #[test]
    fn only_newer_stable_windows_releases_are_eligible() {
        let current = Version::new(2, 9, 0);
        assert!(release(&manifest("v2.10.0"), &current).unwrap().is_some());
        for tag in ["v2.9.0", "v2.9.0+rebuild", "v2.1.0", "v3.0.0-beta.1"] {
            assert!(release(&manifest(tag), &current).unwrap().is_none());
        }
        for key in ["draft", "prerelease"] {
            let mut data = manifest("v3.0.0");
            data[key] = true.into();
            assert!(release(&data, &current).is_err());
        }
        for key in ["digest", "name", "size", "browser_download_url", "state"] {
            let mut data = manifest("v3.0.0");
            data["assets"][0][key] = Value::Null;
            assert!(release(&data, &current).is_err());
        }
    }

    #[test]
    fn redirects_require_github_https() {
        assert!(trusted_redirect(
            &Url::parse("https://release-assets.githubusercontent.com/release").unwrap()
        ));
        for url in [
            "http://github.com/file",
            "https://github.com.invalid/file",
            "https://foo.github.io/file",
            "https://github.com:444/file",
            "https://name@github.com/file",
        ] {
            assert!(!trusted_redirect(&Url::parse(url).unwrap()));
        }
    }

    #[test]
    fn rejects_corrupt_short_and_oversized_files() {
        let bytes = b"VRCLT";
        let digest = format!("{:x}", Sha256::digest(bytes));
        for (size, hash, valid) in [
            (5, digest.as_str(), true),
            (4, &digest, false),
            (6, &digest, false),
            (5, "wrong", false),
        ] {
            let mut copied = Vec::new();
            assert_eq!(
                verified_copy(&bytes[..], &mut copied, size, hash, |_| Ok(())).is_ok(),
                valid
            );
            if valid {
                assert_eq!(copied, bytes);
            }
        }
    }
}
