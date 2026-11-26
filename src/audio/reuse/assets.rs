use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read},
    path::PathBuf,
    time::Duration,
};

const RUNTIME_URL: &str = "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.6/audio-v0.9.6-lib-windows-x64-vulkan-portable.zip";
const MODEL_URL: &str = "https://huggingface.co/audio-cpp/RE-USE-GGUF/resolve/2619b5cc7dcbc0235915104a293d7b94b3b8a1fc/reuse-nemotronlabs-f32.gguf";
pub const MODEL: &str = "reuse-nemotronlabs-f32.gguf";

fn download(
    client: &reqwest::blocking::Client,
    url: &str,
    size: usize,
    hash: &str,
) -> Result<Vec<u8>> {
    let mut data = Vec::with_capacity(size);
    client
        .get(url)
        .send()?
        .error_for_status()?
        .take(size as u64 + 1)
        .read_to_end(&mut data)?;
    ensure!(
        data.len() == size && format!("{:x}", Sha256::digest(&data)) == hash,
        "GPU package verification failed"
    );
    Ok(data)
}

pub fn prepare() -> Result<PathBuf> {
    use std::os::windows::fs::OpenOptionsExt;
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA")?)
        .join("vrclt/models");
    let dir = root.join("nemotron-reuse-v096-f32");
    if dir.is_dir() {
        return Ok(dir);
    }
    fs::create_dir_all(&root)?;
    // Both translation directions can request the same package on first use.
    let _lock = loop {
        match fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(root.join("reuse-install.lock"))
        {
            Ok(file) => break file,
            Err(e) if e.raw_os_error() == Some(32) => {
                std::thread::sleep(Duration::from_millis(200))
            }
            Err(e) => return Err(e.into()),
        }
    };
    if dir.is_dir() {
        return Ok(dir);
    }
    let stage = tempfile::tempdir_in(&root)?;
    let client = reqwest::blocking::Client::builder()
        .user_agent("VRCLT")
        .timeout(Duration::from_secs(150))
        .build()?;
    let archive = download(
        &client,
        RUNTIME_URL,
        32_205_022,
        "e0a1fdac56d6c9f684bcc8e42ae5685813d96f15a9bbcc2a5e66a6b75aec2878",
    )?;
    let mut zip = zip::ZipArchive::new(Cursor::new(archive))?;
    for i in 0..zip.len() {
        let mut item = zip.by_index(i)?;
        let Some(path) = item.enclosed_name() else {
            bail!("Invalid GPU package path")
        };
        if path.parent() == Some(std::path::Path::new("libs"))
            && path.extension().is_some_and(|e| e == "dll")
        {
            let mut f = fs::File::create(stage.path().join(path.file_name().unwrap()))?;
            std::io::copy(&mut item, &mut f)?;
        }
    }
    ensure!(
        stage.path().join("audiocpp.dll").is_file(),
        "Missing GPU runtime"
    );
    let weights = download(
        &client,
        MODEL_URL,
        38_675_712,
        "81f47a42fe9e67e123b9a2c16803193d4d28438432ef4a0a063692615f0913f3",
    )?;
    fs::write(stage.path().join(MODEL), weights)?;
    fs::write(
        stage.path().join("LICENSE-NVIDIA.txt"),
        include_bytes!("../../../licenses/nemotron-reuse.txt"),
    )?;
    fs::write(
        stage.path().join("LICENSE-audio.cpp.txt"),
        include_bytes!("../../../licenses/audiocpp.txt"),
    )?;
    fs::rename(stage.path(), &dir)?;
    Ok(dir)
}
