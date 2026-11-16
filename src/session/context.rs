use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::config::Binding;
use super::wire::Terminal;

pub const SESSION_ENV: &str = "MDRUN_SESSION";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeContext {
    #[serde(with = "native_path")]
    pub cwd: PathBuf,
    pub environment: Vec<(OsString, OsString)>,
    pub umask: u32,
    pub caller_group: i32,
}

pub(super) mod native_path {
    use super::*;

    // Serde's Path encoding requires UTF-8; private POSIX transport must preserve native bytes.
    // https://docs.rs/serde/1.0.229/src/serde/core/ser/impls.rs.html
    pub fn serialize<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        path.as_os_str().serialize(serializer)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<PathBuf, D::Error> {
        OsString::deserialize(deserializer).map(PathBuf::from)
    }
}

impl NativeContext {
    /// Capture native process context before starting threads or changing process state.
    pub fn capture() -> Result<Self, String> {
        // POSIX has no portable read-only umask query; capture before host threads start.
        // https://pubs.opengroup.org/onlinepubs/9799919799/functions/umask.html
        let umask = unsafe {
            let mask = libc::umask(0);
            libc::umask(mask);
            mask
        };
        Ok(Self {
            cwd: std::env::current_dir().map_err(|error| error.to_string())?,
            environment: std::env::vars_os().collect(),
            umask: umask as u32,
            caller_group: unsafe { libc::getpgrp() },
        })
    }

    pub fn apply_binding(
        &mut self,
        binding: &Binding,
        workdir: Option<&Path>,
    ) -> Result<(), String> {
        if let Some(patch) = &binding.context {
            if patch.cwd.is_some() && workdir.is_some() {
                return Err("binding context.cwd conflicts with --workdir".to_owned());
            }
            if let Some(cwd) = &patch.cwd {
                self.cwd = cwd.into();
            }
            for (key, value) in &patch.environment {
                self.environment.retain(|(name, _)| name != key.as_str());
                if let Some(value) = value {
                    self.environment.push((key.into(), value.into()));
                }
            }
            if let Some(umask) = patch.umask {
                self.umask = u32::from(umask);
            }
        }
        if let Some(workdir) = workdir {
            self.cwd = self.cwd.join(workdir);
        }
        Ok(())
    }

    pub fn environment_value(&self, key: &OsStr) -> Option<&OsStr> {
        self.environment
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_os_str())
    }

    pub fn inject(&mut self, directory: &Path, configuration: &Path) -> Result<(), String> {
        let mut paths = vec![directory.to_path_buf()];
        if let Some(path) = self.environment_value(OsStr::new("PATH")) {
            paths.extend(std::env::split_paths(path));
        }
        let path = std::env::join_paths(paths).map_err(|error| error.to_string())?;
        self.environment
            .retain(|(name, _)| name != "PATH" && name != SESSION_ENV);
        self.environment.push(("PATH".into(), path));
        self.environment
            .push((SESSION_ENV.into(), configuration.as_os_str().into()));
        Ok(())
    }

    pub fn remove_injection(&mut self, directory: &Path, baseline: &Self) -> Result<(), String> {
        if let Some(path) = self.environment_value(OsStr::new("PATH")) {
            let mut paths: Vec<_> = std::env::split_paths(path).collect();
            if let Some(index) = paths.iter().position(|path| path == directory) {
                paths.remove(index);
                let path = std::env::join_paths(&paths).map_err(|error| error.to_string())?;
                self.environment.retain(|(name, _)| name != "PATH");
                if !paths.is_empty() || baseline.environment_value(OsStr::new("PATH")).is_some() {
                    self.environment.push(("PATH".into(), path));
                }
            }
        }
        self.environment.retain(|(name, _)| name != SESSION_ENV);
        Ok(())
    }

    pub fn cold_reason(
        &self,
        baseline: &Self,
        binding: &Binding,
        argv: &[OsString],
        terminal: Terminal,
    ) -> Option<&'static str> {
        if terminal.stdin || terminal.stdout || terminal.stderr {
            return Some("terminal");
        }
        if self.cwd.to_str().is_none()
            || argv.iter().any(|value| value.to_str().is_none())
            || self
                .environment
                .iter()
                .any(|(key, value)| key.to_str().is_none() || value.to_str().is_none())
        {
            return Some("native_encoding");
        }
        if binding.cold_if_env_nonempty.iter().any(|key| {
            self.environment_value(OsStr::new(key))
                .is_some_and(|value| !value.is_empty())
        }) {
            return Some("startup_options");
        }
        if self.cwd != baseline.cwd {
            return Some("cwd");
        }
        if self.umask != baseline.umask {
            return Some("umask");
        }
        let actual: BTreeMap<_, _> = self
            .environment
            .iter()
            .map(|(key, value)| (key, value))
            .collect();
        let expected: BTreeMap<_, _> = baseline
            .environment
            .iter()
            .map(|(key, value)| (key, value))
            .collect();
        if actual != expected {
            return Some("environment");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    fn context() -> NativeContext {
        NativeContext {
            cwd: PathBuf::from("/workspace"),
            environment: vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("PWD".into(), "/logical/workspace".into()),
                ("VALUE".into(), "original".into()),
            ],
            umask: 0o022,
            caller_group: 12,
        }
    }

    fn binding() -> Binding {
        serde_json::from_value(serde_json::json!({"version":1,"name":"tool","artifact_id":"fixed","worker_argv":["/worker"],"cold_argv":["/cold"],"cold_if_env_nonempty":["STARTUP_OPTIONS"]})).unwrap()
    }

    fn terminal() -> Terminal {
        Terminal {
            stdin: false,
            stdout: false,
            stderr: false,
        }
    }

    #[test]
    fn injection_roundtrips_unset_empty_and_native_paths_without_changing_other_context() {
        for path in [
            None,
            Some(OsString::new()),
            Some(OsString::from("/usr/bin:/bin")),
            Some(OsString::from_vec(b"/path-\xff:/bin".to_vec())),
        ] {
            let mut original = context();
            original.environment.retain(|(name, _)| name != "PATH");
            if let Some(path) = path {
                original.environment.push(("PATH".into(), path));
            }
            let mut modified = original.clone();
            modified
                .inject(Path::new("/session/bin"), Path::new("/session/client.json"))
                .unwrap();
            assert_eq!(
                modified.environment_value(OsStr::new(SESSION_ENV)),
                Some(OsStr::new("/session/client.json"))
            );
            modified
                .remove_injection(Path::new("/session/bin"), &original)
                .unwrap();
            assert_eq!(
                modified.cold_reason(&original, &binding(), &["argument".into()], terminal()),
                if original
                    .environment
                    .iter()
                    .any(|(_, value)| value.to_str().is_none())
                {
                    Some("native_encoding")
                } else {
                    None
                }
            );
            assert_eq!(
                modified.environment_value(OsStr::new("PATH")),
                original.environment_value(OsStr::new("PATH"))
            );
            assert_eq!(modified.cwd, original.cwd);
            assert_eq!(modified.umask, original.umask);
        }
    }

    #[test]
    fn only_session_injection_is_removed_and_real_context_changes_route_cold() {
        let original = context();
        let mut changed = original.clone();
        changed
            .inject(Path::new("/session/bin"), Path::new("/session/client.json"))
            .unwrap();
        changed.environment.retain(|(name, _)| name != "PATH");
        changed
            .environment
            .push(("PATH".into(), "/other:/session/bin:/usr/bin:/bin".into()));
        changed
            .remove_injection(Path::new("/session/bin"), &original)
            .unwrap();
        assert_eq!(
            changed.environment_value(OsStr::new("PATH")),
            Some(OsStr::new("/other:/usr/bin:/bin"))
        );
        assert_eq!(
            changed.cold_reason(&original, &binding(), &[], terminal()),
            Some("environment")
        );
        let mut changed = original.clone();
        changed.cwd = "/different".into();
        assert_eq!(
            changed.cold_reason(&original, &binding(), &[], terminal()),
            Some("cwd")
        );
        let mut changed = original.clone();
        changed.umask = 0o077;
        assert_eq!(
            changed.cold_reason(&original, &binding(), &[], terminal()),
            Some("umask")
        );
        for key in ["PWD", "VALUE", "SHLVL", "_"] {
            let mut changed = original.clone();
            changed.environment.retain(|(name, _)| name != key);
            changed.environment.push((key.into(), "changed".into()));
            assert_eq!(
                changed.cold_reason(&original, &binding(), &[], terminal()),
                Some("environment")
            );
        }
    }

    #[test]
    fn terminal_native_encoding_and_startup_options_refuse_warm_before_execution() {
        let original = context();
        assert_eq!(
            original.cold_reason(
                &original,
                &binding(),
                &[],
                Terminal {
                    stdin: true,
                    ..terminal()
                }
            ),
            Some("terminal")
        );
        assert_eq!(
            original.cold_reason(
                &original,
                &binding(),
                &[OsString::from_vec(vec![0xff])],
                terminal()
            ),
            Some("native_encoding")
        );
        let mut changed = original.clone();
        changed
            .environment
            .push(("STARTUP_OPTIONS".into(), "-flag".into()));
        assert_eq!(
            changed.cold_reason(&changed, &binding(), &[], terminal()),
            Some("startup_options")
        );
        changed.environment.pop();
        changed
            .environment
            .push(("STARTUP_OPTIONS".into(), "".into()));
        assert_eq!(
            changed.cold_reason(&changed, &binding(), &[], terminal()),
            None
        );
    }

    #[test]
    fn explicit_context_patch_retains_pwd_and_unrelated_environment() {
        let mut binding = binding();
        binding.context = serde_json::from_value(serde_json::json!({"cwd":"/configured","environment":{"VALUE":null,"NEW":"chosen"},"umask":63})).unwrap();
        let mut actual = context();
        actual.apply_binding(&binding, None).unwrap();
        assert_eq!(actual.cwd, PathBuf::from("/configured"));
        assert_eq!(
            actual.environment_value(OsStr::new("PWD")),
            Some(OsStr::new("/logical/workspace"))
        );
        assert_eq!(
            actual.environment_value(OsStr::new("NEW")),
            Some(OsStr::new("chosen"))
        );
        assert_eq!(actual.environment_value(OsStr::new("VALUE")), None);
        assert_eq!(actual.umask, 63);
        assert!(
            actual
                .apply_binding(&binding, Some(Path::new("/other")))
                .is_err()
        );
    }

    #[test]
    fn native_directory_bytes_survive_private_context_transport() {
        let mut original = context();
        original.cwd = PathBuf::from(OsString::from_vec(b"/directory-\xff".to_vec()));
        let encoded = serde_json::to_vec(&original).expect("native cwd must reach cold routing");
        let restored: NativeContext = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.cwd, original.cwd);
        assert_eq!(
            restored.cold_reason(&original, &binding(), &[], terminal()),
            Some("native_encoding")
        );
    }
}
