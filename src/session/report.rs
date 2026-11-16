use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub(crate) struct ReportFile {
    temporary: PathBuf,
    destination: PathBuf,
    file: File,
}

impl ReportFile {
    pub(crate) fn prepare(destination: PathBuf) -> Result<Self, String> {
        if destination.is_dir() {
            return Err("execution report is a directory".to_owned());
        }
        let parent = destination
            .parent()
            .ok_or("execution report has no parent")?;
        let temporary = parent.join(format!(".mdrun-report-{}", uuid::Uuid::new_v4().simple()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| "cannot prepare execution report")?;
        let prepared = Self {
            temporary,
            destination,
            file,
        };
        prepared
            .file
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| "cannot protect execution report")?;
        Ok(prepared)
    }

    pub(crate) fn invalidate_previous(&self) -> Result<(), String> {
        match fs::remove_file(&self.destination) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("cannot invalidate previous execution report".to_owned()),
        }
    }

    pub(crate) fn publish(&mut self, report: &serde_json::Value) -> Result<(), String> {
        let encoded =
            serde_json::to_vec_pretty(report).map_err(|_| "cannot write execution report")?;
        self.file
            .write_all(&encoded)
            .map_err(|_| "cannot write execution report")?;
        fs::rename(&self.temporary, &self.destination)
            .map_err(|_| "cannot publish execution report")?;
        Ok(())
    }
}

impl Drop for ReportFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.temporary);
    }
}

pub(crate) fn conflicts(report: &Path, documents: &[PathBuf]) -> Result<bool, String> {
    let resolved = resolve_destination(report)?;
    let metadata = fs::metadata(&resolved).ok();
    for document in documents {
        if resolve_destination(document)? == resolved
            || metadata.as_ref().is_some_and(|report| {
                fs::metadata(document)
                    .is_ok_and(|input| input.dev() == report.dev() && input.ino() == report.ino())
            })
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn resolve_destination(report: &Path) -> Result<PathBuf, String> {
    let mut path = report.to_owned();
    for _ in 0..40 {
        if let Ok(target) = fs::read_link(&path) {
            path = if target.is_absolute() {
                target
            } else {
                path.parent()
                    .ok_or("execution report has no parent")?
                    .join(target)
            };
        } else if path.exists() {
            return fs::canonicalize(path)
                .map_err(|_| "cannot resolve execution report".to_owned());
        } else {
            let parent = path.parent().ok_or("execution report has no parent")?;
            return Ok(fs::canonicalize(parent)
                .map_err(|_| "cannot resolve execution report directory")?
                .join(path.file_name().ok_or("execution report has no filename")?));
        }
    }
    Err("execution report symlink chain cannot be resolved".to_owned())
}
