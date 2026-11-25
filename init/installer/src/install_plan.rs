//! Stage the candidate and retain the previous files until publication is committed.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};

const STAGE: &str = ".stellarr-install-stage";
const MANAGED: &[&str] = &[
    "docker-compose.turn.yml",
    "docker-compose.yml",
    ".env.example",
    ".env",
];
const JOURNAL: &str = "pending";
const OWNER: &[u8] = b"Stellarr installer staging directory\n";

fn path_id(identity: &[u8]) -> u64 {
    // The same installation uses the same identifier across installer builds.
    identity.iter().fold(0xcbf29ce484222325_u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

pub fn lock(scope: &str, identity: &[u8]) -> io::Result<File> {
    let id = path_id(identity);
    let directory = PathBuf::from("/tmp");
    #[cfg(feature = "test-seams")]
    let directory = std::env::var_os("STELLARR_INSTALLER_TEST_LOCKS")
        .map(PathBuf::from)
        .unwrap_or(directory);
    let path = directory.join(format!("stellarr-installer-{scope}-{id:016x}.lock"));
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = match options.open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match options
                .clone()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .open(&path)
            {
                Ok(file) => {
                    file.set_permissions(fs::Permissions::from_mode(0o644))?;
                    file
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    options.open(&path)?
                }
                Err(error) => return Err(error),
            }
        }
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("installer lock is not a regular file"));
    }
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "another installer is already running",
            ));
        }
        Err(TryLockError::Error(error)) => return Err(error),
    }
    Ok(file)
}

pub struct InstallPlan {
    pub path: PathBuf,
    root: PathBuf,
    live: PathBuf,
    changed: Vec<(&'static str, Option<Vec<u8>>)>,
}

pub fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => fs::read(path).map(Some),
        Ok(_) => Err(io::Error::other(format!(
            "{} is not a regular file",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::symlink_metadata(path) {
            Ok(_) => Err(error),
            Err(metadata) if metadata.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(metadata) => Err(metadata),
        },
        Err(error) => Err(error),
    }
}

impl InstallPlan {
    pub fn new(live: &Path) -> io::Result<Self> {
        let plan = Self::at(live);
        let seed = live.join(format!(
            "{STAGE}.{}-{:016x}",
            std::process::id(),
            crate::sys::unique()
        ));
        fs::DirBuilder::new().mode(0o700).create(&seed)?;
        let prepared = Self::write(&seed.join("owner"), OWNER, 0o600)
            .and_then(|()| fs::create_dir(seed.join("project")))
            .and_then(|()| sync_directory(&seed))
            .and_then(|()| fs::rename(&seed, &plan.root));
        match prepared {
            Ok(()) => Ok(plan),
            Err(error) => {
                let _ = fs::remove_dir_all(&seed);
                Err(error)
            }
        }
    }

    fn at(live: &Path) -> Self {
        let root = live.join(STAGE);
        Self {
            path: root.join("project"),
            root,
            live: live.to_path_buf(),
            changed: Vec::new(),
        }
    }

    pub fn recover(live: &Path) -> io::Result<bool> {
        let plan = Self::at(live);
        match fs::symlink_metadata(&plan.root) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
            Ok(meta) if !meta.is_dir() => {
                return Err(io::Error::other(
                    "installer staging path is not a directory",
                ));
            }
            Ok(_) => {}
        }
        if read(&plan.root.join("owner"))?.as_deref() != Some(OWNER) {
            return Err(io::Error::other("unrecognized installer staging directory"));
        }
        let journal = read(&plan.root.join(JOURNAL))?;
        if let Some(journal) = &journal {
            let journal = std::str::from_utf8(journal).map_err(io::Error::other)?;
            for name in journal.lines() {
                if !MANAGED.contains(&name) {
                    return Err(io::Error::other("unrecognized file in installer journal"));
                }
                let old = plan.root.join("before").join(name);
                let dest = live.join(name);
                match fs::symlink_metadata(&old) {
                    Ok(_) => {
                        let restore = plan.root.join("restore");
                        remove_file_if_present(&restore)?;
                        fs::hard_link(old, &restore)?;
                        fs::rename(restore, dest)?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        remove_file_if_present(&dest)?
                    }
                    Err(error) => return Err(error),
                }
            }
            sync_directory(live)?;
            fs::remove_file(plan.root.join(JOURNAL))?;
            sync_directory(&plan.root)?;
        }
        plan.cleanup()?;
        Ok(journal.is_some())
    }

    fn view_link(&self) -> Option<PathBuf> {
        self.live.parent().map(|parent| {
            let id = path_id(self.live.as_os_str().as_encoded_bytes());
            parent.join(format!(".stellarr-install-view-{id:016x}"))
        })
    }

    /// The alias sits next to the real project, so parent-relative override
    /// references keep their meaning. The candidate remains on the same
    /// filesystem as the files it will replace.
    pub fn view(&self) -> PathBuf {
        if let Some(link) = self.view_link()
            && symlink(&self.path, &link).is_ok()
        {
            return link;
        }
        self.path.clone()
    }

    fn cleanup(&self) -> io::Result<()> {
        if let Some(link) = self.view_link()
            && fs::read_link(&link).ok().as_deref() == Some(&self.path)
        {
            fs::remove_file(link)?;
        }
        fs::remove_dir_all(&self.root)
    }

    pub fn mirror(&self) -> io::Result<()> {
        for entry in fs::read_dir(&self.live)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == STAGE || self.path.join(&name).symlink_metadata().is_ok() {
                continue;
            }
            symlink(entry.path(), self.path.join(name))?;
        }
        Ok(())
    }

    fn write(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(path)?;
        file.write_all(bytes)?;
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.sync_all()
    }

    pub fn candidate(
        &mut self,
        name: &'static str,
        bytes: &[u8],
        mode: u32,
        managed: bool,
    ) -> io::Result<()> {
        Self::write(&self.path.join(name), bytes, mode)?;
        if managed {
            let previous = read(&self.live.join(name))?;
            if previous.as_deref() != Some(bytes) {
                self.changed.push((name, previous));
            }
        }
        Ok(())
    }

    pub fn build_context_path(live: &Path) -> PathBuf {
        live.join(STAGE).join("build-context.json")
    }

    pub fn set_build_context(&self) -> io::Result<()> {
        let context = self
            .live
            .to_str()
            .ok_or_else(|| io::Error::other("checkout path is not UTF-8"))?;
        let overlay = serde_json::json!({"services": {"stellarr": {"build": {
            "context": context.replace('$', "$$"),
        }}}});
        Self::write(
            &Self::build_context_path(&self.live),
            overlay.to_string().as_bytes(),
            0o600,
        )
    }

    pub fn build_paths_file(live: &Path) -> PathBuf {
        live.join(STAGE).join("build-paths.json")
    }

    pub fn write_build_paths(
        &self,
        services: serde_json::Map<String, serde_json::Value>,
    ) -> io::Result<()> {
        let overlay = serde_json::json!({"services": services});
        Self::write(
            &Self::build_paths_file(&self.live),
            overlay.to_string().as_bytes(),
            0o600,
        )
    }

    pub fn publish(&self, stamp: &str) -> io::Result<Vec<(&'static str, String)>> {
        if self.changed.is_empty() {
            return Ok(Vec::new());
        }
        let before = self.root.join("before");
        fs::create_dir(&before)?;
        let mut backups = Vec::new();
        for (name, expected) in &self.changed {
            if read(&self.live.join(name))? != *expected {
                return Err(io::Error::other(format!(
                    "{name} changed during installation"
                )));
            }
        }
        for &(name, _) in &self.changed {
            let source = self.live.join(name);
            match fs::symlink_metadata(&source) {
                Ok(_) => fs::hard_link(&source, before.join(name))?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            }
            if name != ".env.example" {
                let backup = self.backup(&source, name, stamp)?;
                backups.push((name, backup));
            }
        }
        sync_directory(&before)?;
        let journal = self
            .changed
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join("\n");
        Self::write(&self.root.join("journal.tmp"), journal.as_bytes(), 0o600)?;
        fs::rename(self.root.join("journal.tmp"), self.root.join(JOURNAL))?;
        sync_directory(&self.root)?;
        sync_directory(&self.live)?;
        for &name in MANAGED {
            if self.changed.iter().any(|(changed, _)| changed == &name) {
                fs::rename(self.path.join(name), self.live.join(name))?;
            }
        }
        sync_directory(&self.live)?;
        Ok(backups)
    }

    fn backup(&self, source: &Path, name: &str, stamp: &str) -> io::Result<String> {
        let input = File::open(source)?;
        let meta = input.metadata()?;
        for suffix in 0.. {
            let backup = if suffix == 0 {
                format!("{name}.bak-{stamp}")
            } else {
                format!("{name}.bak-{stamp}-{suffix}")
            };
            let mut output = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(self.live.join(&backup))
            {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            };
            let result = io::copy(&mut &input, &mut output)
                .and_then(|_| {
                    output.set_permissions(fs::Permissions::from_mode(if name == ".env" {
                        0o600
                    } else {
                        meta.permissions().mode() & 0o777
                    }))
                })
                .and_then(|_| output.set_modified(meta.modified()?))
                .and_then(|_| output.sync_all());
            if let Err(error) = result {
                let _ = fs::remove_file(self.live.join(&backup));
                return Err(error);
            }
            return Ok(backup);
        }
        unreachable!()
    }

    pub fn commit(&self) -> io::Result<()> {
        remove_file_if_present(&self.root.join(JOURNAL))?;
        sync_directory(&self.root)?;
        self.cleanup()
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    let directory = File::open(path)?;
    #[cfg(not(target_os = "macos"))]
    return directory.sync_all();

    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        // File::sync_all uses F_FULLFSYNC on Darwin. Directory metadata
        // uses fsync, while staged file payloads still use F_FULLFSYNC.
        loop {
            // SAFETY: directory owns the descriptor until fsync returns.
            if unsafe { libc::fsync(directory.as_raw_fd()) } == 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

fn remove_file_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
