use std::io;

pub(super) fn live_group_members(group: libc::pid_t) -> io::Result<Vec<libc::pid_t>> {
    if group <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid owned group",
        ));
    }
    observe_group(group)
}

#[cfg(target_os = "macos")]
fn observe_group(group: libc::pid_t) -> io::Result<Vec<libc::pid_t>> {
    // Apple's libproc returns zero for both an empty list and a failed syscall.
    // https://github.com/apple-oss-distributions/xnu/blob/main/libsyscall/wrappers/libproc/libproc.c
    const PROC_PGRP_ONLY: u32 = 2;
    let mut capacity = 64;
    let pids = loop {
        let mut pids = vec![0 as libc::pid_t; capacity];
        let bytes = size_of_val(pids.as_slice());
        unsafe { *libc::__error() = 0 };
        let count = unsafe {
            libc::proc_listpids(
                PROC_PGRP_ONLY,
                group as u32,
                pids.as_mut_ptr().cast(),
                bytes as i32,
            )
        };
        if count < 0 || (count == 0 && io::Error::last_os_error().raw_os_error() != Some(0)) {
            return Err(io::Error::last_os_error());
        }
        if count as usize >= bytes {
            capacity *= 2;
            if capacity > 1_048_576 {
                return Err(io::Error::other("owned group enumeration exceeds capacity"));
            }
            continue;
        }
        if !(count as usize).is_multiple_of(size_of::<libc::pid_t>()) {
            return Err(io::Error::other("invalid native group enumeration"));
        }
        pids.truncate(count as usize / size_of::<libc::pid_t>());
        break pids;
    };
    let mut live = Vec::new();
    for pid in pids.into_iter().filter(|pid| *pid > 0) {
        // Group/state need no full BSD record, whose same-user check rejects unrelated group peers.
        // https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.81.4/bsd/kern/proc_info.c#L2048-L2063
        let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdshortinfo>() };
        unsafe { *libc::__error() = 0 };
        let bytes = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDT_SHORTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdshortinfo).cast(),
                size_of_val(&info) as i32,
            )
        };
        let error = io::Error::last_os_error();
        let errno = error.raw_os_error().unwrap_or(0);
        if bytes == 0 && errno == libc::ESRCH {
            continue;
        }
        if bytes as usize != size_of_val(&info) {
            return Err(io::Error::new(
                if errno == 0 {
                    io::ErrorKind::InvalidData
                } else {
                    error.kind()
                },
                format!(
                    "cannot inspect owned group member {pid} in group {group}: proc_pidinfo returned {bytes} bytes, expected {}; errno {errno}",
                    size_of_val(&info)
                ),
            ));
        }
        if info.pbsi_pgid == group as u32 && info.pbsi_status != libc::SZOMB {
            live.push(pid);
        }
    }
    Ok(live)
}

#[cfg(target_os = "linux")]
fn observe_group(group: libc::pid_t) -> io::Result<Vec<libc::pid_t>> {
    let mut live = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        let stat = match std::fs::read(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(error) if process_disappeared(&error) => continue,
            Err(error) => return Err(error),
        };
        let (process_group, state) = parse_linux_stat(&stat)?;
        if process_group == group && !matches!(state, b'Z' | b'X' | b'x') {
            live.push(pid);
        }
    }
    Ok(live)
}

#[cfg(any(target_os = "linux", test))]
fn process_disappeared(error: &io::Error) -> bool {
    // A task can exit after procfs open succeeds, making the subsequent read fail with ESRCH.
    // https://docs.kernel.org/filesystems/proc.html#process-specific-subdirectories
    matches!(error.raw_os_error(), Some(libc::ENOENT) | Some(libc::ESRCH))
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_stat(stat: &[u8]) -> io::Result<(libc::pid_t, u8)> {
    let invalid = || io::Error::other("invalid procfs process status");
    // comm is parenthesized but may itself contain spaces or parentheses.
    // https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html
    let end = stat
        .iter()
        .rposition(|byte| *byte == b')')
        .ok_or_else(invalid)?;
    let mut fields = stat[end + 1..]
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|part| !part.is_empty());
    let state = fields
        .next()
        .filter(|field| field.len() == 1)
        .ok_or_else(invalid)?[0];
    fields.next().ok_or_else(invalid)?;
    let group = std::str::from_utf8(fields.next().ok_or_else(invalid)?)
        .map_err(|_| invalid())?
        .parse::<libc::pid_t>()
        .map_err(|_| invalid())?;
    // do_task_stat can retain its -1 pgid sentinel after a dead task loses sighand.
    // https://github.com/torvalds/linux/blob/v6.12/fs/proc/array.c#L442-L509
    if group < 0 && !(group == -1 && matches!(state, b'Z' | b'X' | b'x')) {
        return Err(invalid());
    }
    Ok((group, state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_live_process_is_in_its_observed_group() {
        let group = unsafe { libc::getpgrp() };
        let pid = unsafe { libc::getpid() };
        assert!(live_group_members(group).unwrap().contains(&pid));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_group_is_observable_without_full_bsd_process_records() {
        let group = unsafe { libc::getpgid(1) };
        assert!(
            group > 0,
            "cannot independently identify the system process group"
        );
        assert!(live_group_members(group).unwrap().contains(&1));
    }

    #[test]
    fn invalid_group_is_not_a_cleanup_certificate() {
        assert!(live_group_members(0).is_err());
        assert!(live_group_members(-1).is_err());
    }

    #[test]
    fn procfs_disappearance_accepts_enoent_and_esrch_only() {
        for errno in [libc::ENOENT, libc::ESRCH] {
            assert!(
                process_disappeared(&io::Error::from_raw_os_error(errno)),
                "errno {errno}"
            );
        }
        for errno in [
            libc::EACCES,
            libc::EPERM,
            libc::EIO,
            libc::EBADF,
            libc::EINVAL,
            libc::EINTR,
        ] {
            assert!(
                !process_disappeared(&io::Error::from_raw_os_error(errno)),
                "errno {errno}"
            );
        }
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::InvalidData,
            io::ErrorKind::Other,
        ] {
            assert!(!process_disappeared(&io::Error::new(
                kind,
                "observation failed"
            )));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn retained_procfs_stat_reports_disappearance_after_owned_child_exit() {
        use std::io::Read;
        use std::os::unix::process::ExitStatusExt;
        use std::process::{Command, Stdio};

        let mut child = Command::new("/bin/sleep")
            .arg("15")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stat = std::fs::File::open(format!("/proc/{}/stat", child.id()));
        let killed = child.kill();
        let reaped = child.wait();
        let mut stat = stat.unwrap();
        killed.unwrap();
        assert_eq!(reaped.unwrap().signal(), Some(libc::SIGKILL));
        let error = stat.read_to_end(&mut Vec::new()).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ESRCH));
        assert!(process_disappeared(&error));
    }

    #[test]
    fn procfs_names_with_parentheses_do_not_change_group_fields() {
        assert_eq!(
            parse_linux_stat(b"90 (a name ) with () S 1 83 82 0").unwrap(),
            (83, b'S')
        );
        assert_eq!(
            parse_linux_stat(b"90 (gone) Z 1 83 82 0").unwrap(),
            (83, b'Z')
        );
        assert!(parse_linux_stat(b"90 (bad) S 1 invalid").is_err());
    }

    #[test]
    fn ungrouped_kernel_processes_do_not_invalidate_procfs_observation() {
        assert_eq!(
            parse_linux_stat(b"2 (kthreadd) S 0 0 0 0").unwrap(),
            (0, b'S')
        );
    }

    #[test]
    fn dead_procfs_records_can_have_already_detached_process_groups() {
        for state in *b"ZXx" {
            let record = format!("7313 (sleep) {} 0 -1 -1 0 -1", state as char);
            assert_eq!(parse_linux_stat(record.as_bytes()).unwrap(), (-1, state));
        }
        for state in *b"RSDTI" {
            let record = format!("7313 (sleep) {} 0 -1 -1 0 -1", state as char);
            assert!(parse_linux_stat(record.as_bytes()).is_err());
        }
        assert!(parse_linux_stat(b"7313 (sleep) X 0 invalid -1").is_err());
        assert!(parse_linux_stat(b"7313 (sleep) X 0 -2 -1").is_err());
    }

    #[test]
    fn exited_unreaped_anchor_does_not_count_as_live_resource_owner() {
        let child = unsafe { libc::fork() };
        assert!(child >= 0);
        if child == 0 {
            unsafe {
                libc::setpgid(0, 0);
                libc::_exit(0);
            }
        }
        let mut status = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
        assert_eq!(
            unsafe {
                libc::waitid(
                    libc::P_PID,
                    child as libc::id_t,
                    &mut status,
                    libc::WEXITED | libc::WNOWAIT,
                )
            },
            0
        );
        let observed = live_group_members(child);
        assert_eq!(
            unsafe { libc::waitpid(child, std::ptr::null_mut(), 0) },
            child
        );
        assert!(observed.unwrap().is_empty());
    }
}
