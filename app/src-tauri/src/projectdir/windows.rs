//! Keep every directory component open without FILE_SHARE_DELETE while mutating its children.
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

pub(super) struct Directory {
    path: PathBuf,
    _chain: Vec<File>,
}

impl Directory {
    pub(super) fn open(path: &Path, create: bool) -> io::Result<Self> {
        let mut cursor = PathBuf::new();
        let mut chain = Vec::new();
        for part in path.components() {
            cursor.push(part.as_os_str());
            match part {
                Component::Prefix(_) | Component::RootDir => continue,
                Component::Normal(_) => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "non-absolute directory path",
                    ));
                }
            }
            if create {
                match std::fs::create_dir(&cursor) {
                    Err(error) if error.kind() != io::ErrorKind::AlreadyExists => {
                        return Err(error);
                    }
                    _ => {}
                }
            }
            let file = OpenOptions::new()
                .read(true)
                .share_mode(1 | 2) // FILE_SHARE_READ | FILE_SHARE_WRITE, deliberately no DELETE.
                .custom_flags(0x02000000 | 0x00200000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT.
                .open(&cursor)?;
            let metadata = file.metadata()?;
            if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
                return Err(io::Error::other(
                    "project directory is a reparse point or not a directory",
                ));
            }
            chain.push(file);
        }
        Ok(Self {
            path: path.to_owned(),
            _chain: chain,
        })
    }

    pub(super) fn verify(&self, path: &Path) -> io::Result<()> {
        Self::open(path, false).map(|_| ())
    }

    pub(super) fn create_temp(&self, name: &OsStr) -> io::Result<File> {
        // create_new maps to CREATE_NEW and refuses an existing final component.
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(0x00200000)
            .open(self.path.join(name))?;
        if file.metadata()?.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("temporary file is a reparse point"));
        }
        Ok(file)
    }

    pub(super) fn rename(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
        std::fs::rename(self.path.join(from), self.path.join(to))
    }

    pub(super) fn remove(&self, name: &OsStr) -> io::Result<()> {
        let path = self.path.join(name);
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("project path is a reparse point"));
        }
        if metadata.is_dir() {
            let child = Self::open(&path, false)?;
            for entry in std::fs::read_dir(&path)? {
                child.remove(&entry?.file_name())?;
            }
            child.verify(&path)?;
            drop(child);
            std::fs::remove_dir(path)
        } else {
            std::fs::remove_file(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn directory_locks_exclusive_creation_and_reparse_refusal() {
        let root =
            std::env::temp_dir().join(format!("simrep-win-directory-{}", std::process::id()));
        let inside = root.join("inside");
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"KEEP").unwrap();
        let dir = Directory::open(&inside, true).unwrap();
        dir.verify(&inside).unwrap();
        assert!(std::fs::rename(&inside, root.join("moved")).is_err());
        let mut file = dir.create_temp(OsStr::new("new.tmp")).unwrap();
        assert!(dir.create_temp(OsStr::new("new.tmp")).is_err());
        file.write_all(b"SAFE").unwrap();
        drop(file);
        dir.rename(OsStr::new("new.tmp"), OsStr::new("new.json"))
            .unwrap();
        assert_eq!(std::fs::read(inside.join("new.json")).unwrap(), b"SAFE");
        dir.remove(OsStr::new("new.json")).unwrap();
        drop(dir);

        let junction = root.join("junction.tmp");
        let result = std::process::Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        let dir = Directory::open(&root, false).unwrap();
        assert!(Directory::open(&junction.join("nested"), true).is_err());
        assert!(!outside.join("nested").exists());
        assert!(dir.create_temp(OsStr::new("junction.tmp")).is_err());
        assert!(dir.remove(OsStr::new("junction.tmp")).is_err());
        assert_eq!(std::fs::read(outside.join("sentinel")).unwrap(), b"KEEP");
        dir.remove(OsStr::new("inside")).unwrap();
        drop(dir);
        std::fs::remove_dir(&junction).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
