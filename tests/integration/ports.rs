use std::net::{SocketAddr, TcpListener};
use std::sync::{Mutex, MutexGuard};

// The live socket excludes other bind(2) callers during preparation; this gate
// excludes this binary's other fixture allocations and probes during close-to-ready handoff.
// Never reuse candidates: an earlier fixture may still probe after releasing its listener.
static ALLOCATION: Mutex<u32> = Mutex::new(1024);

fn automatic_port_range() -> std::io::Result<(u16, u16)> {
    #[cfg(target_os = "linux")]
    {
        parse_port_range(&std::fs::read_to_string(
            "/proc/sys/net/ipv4/ip_local_port_range",
        )?)
    }
    #[cfg(target_os = "macos")]
    {
        let read = |name: &std::ffi::CStr| -> std::io::Result<u16> {
            let mut value: libc::c_int = 0;
            let mut size = size_of_val(&value);
            if unsafe {
                libc::sysctlbyname(
                    name.as_ptr(),
                    (&mut value as *mut libc::c_int).cast(),
                    &mut size,
                    std::ptr::null_mut(),
                    0,
                )
            } < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if size != size_of_val(&value) {
                return Err(std::io::Error::other(
                    "invalid automatic port range sysctl size",
                ));
            }
            u16::try_from(value).map_err(std::io::Error::other)
        };
        let bounds = [
            read(c"net.inet.ip.portrange.first")?,
            read(c"net.inet.ip.portrange.last")?,
            read(c"net.inet.ip.portrange.hifirst")?,
            read(c"net.inet.ip.portrange.hilast")?,
        ];
        Ok((*bounds.iter().min().unwrap(), *bounds.iter().max().unwrap()))
    }
}

fn parse_port_range(input: &str) -> std::io::Result<(u16, u16)> {
    let fields: Vec<_> = input.split_whitespace().collect();
    if fields.len() == 2
        && let (Ok(first), Ok(last)) = (fields[0].parse::<u16>(), fields[1].parse::<u16>())
        && first <= last
    {
        return Ok((first, last));
    }
    Err(std::io::Error::other("invalid automatic port range"))
}

fn reserve_endpoint(next: &mut u32, automatic: (u16, u16)) -> std::io::Result<TcpListener> {
    // bind(0) and outgoing connect share the automatic range, outside our userspace gate.
    // https://docs.kernel.org/networking/ip-sysctl.html#ip-variables
    while *next <= u32::from(u16::MAX) {
        let port = *next as u16;
        *next += 1;
        if (automatic.0..=automatic.1).contains(&port) {
            continue;
        }
        match TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)) {
            Ok(listener) => return Ok(listener),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AddrNotAvailable,
        format!(
            "no unused bindable test port outside automatic range {}..={} (candidate space exhausted)",
            automatic.0, automatic.1
        ),
    ))
}

pub(super) struct Reservation {
    listener: Option<TcpListener>,
    address: SocketAddr,
    _allocation: MutexGuard<'static, u32>,
}

impl Reservation {
    pub(super) fn new() -> Self {
        let mut allocation = ALLOCATION.lock().unwrap_or_else(|error| error.into_inner());
        let automatic = automatic_port_range().expect("cannot observe the OS automatic port range");
        let listener = reserve_endpoint(&mut allocation, automatic).unwrap();
        let address = listener.local_addr().unwrap();
        Self {
            listener: Some(listener),
            address,
            _allocation: allocation,
        }
    }

    pub(super) fn address(&self) -> SocketAddr {
        self.address
    }

    pub(super) fn probe(&self) -> std::io::Result<()> {
        // This reservation already owns ALLOCATION; taking it again would deadlock.
        TcpListener::bind(self.address).map(drop)
    }

    pub(super) fn release_listener(&mut self) {
        assert!(
            self.listener.take().is_some(),
            "listener already handed off"
        );
    }

    pub(super) fn handoff<T>(mut self, start_and_wait_ready: impl FnOnce(SocketAddr) -> T) -> T {
        self.release_listener();
        start_and_wait_ready(self.address)
    }
}

pub(super) fn listener() -> TcpListener {
    let mut reservation = Reservation::new();
    reservation.listener.take().unwrap()
}

pub(super) fn bind(address: SocketAddr) -> std::io::Result<TcpListener> {
    let _allocation = ALLOCATION.lock().unwrap_or_else(|error| error.into_inner());
    TcpListener::bind(address)
}

pub(super) fn diagnostics(address: SocketAddr) -> String {
    #[cfg(target_os = "linux")]
    {
        let mut sockets = Vec::new();
        for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
            match std::fs::read_to_string(table) {
                Ok(contents) => {
                    for line in contents.lines().skip(1) {
                        let fields: Vec<_> = line.split_whitespace().collect();
                        if fields.len() > 9
                            && fields[1].rsplit_once(':').is_some_and(|(_, port)| {
                                u16::from_str_radix(port, 16).ok() == Some(address.port())
                            })
                        {
                            sockets.push((table, fields[3].to_owned(), fields[9].to_owned()));
                        }
                    }
                }
                Err(error) => return format!("endpoint {address}; {table}: {error}"),
            }
        }
        let mut owners = Vec::new();
        if let Ok(processes) = std::fs::read_dir("/proc") {
            for process in processes.flatten() {
                let Some(pid) = process
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<u32>().ok())
                else {
                    continue;
                };
                let Ok(descriptors) = std::fs::read_dir(process.path().join("fd")) else {
                    continue;
                };
                for descriptor in descriptors.flatten() {
                    let Ok(target) = std::fs::read_link(descriptor.path()) else {
                        continue;
                    };
                    for (_, _, inode) in &sockets {
                        if inode != "0"
                            && target.as_os_str() == format!("socket:[{inode}]").as_str()
                        {
                            owners.push((pid, inode.clone()));
                        }
                    }
                }
            }
        }
        format!(
            "endpoint {address}; TCP (table, state, inode): {sockets:?}; owners (pid, inode): {owners:?}"
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        format!("endpoint {address}")
    }
}

#[test]
fn automatic_port_range_requires_two_ordered_native_port_bounds() {
    assert_eq!(parse_port_range("32768\t60999\n").unwrap(), (32768, 60999));
    assert_eq!(parse_port_range("0 65535").unwrap(), (0, 65535));
    for input in [
        "",
        "32768",
        "32768 60999 65535",
        "-1 65535",
        "0 65536",
        "60999 32768",
        "first last",
    ] {
        assert!(parse_port_range(input).is_err(), "{input:?}");
    }
}

#[test]
fn reserved_endpoints_are_outside_the_observed_automatic_port_range() {
    let (first, last) = automatic_port_range().unwrap();
    let reservation = Reservation::new();
    assert!(!(first..=last).contains(&reservation.address().port()));
}

#[test]
fn released_fixture_endpoints_are_not_reassigned_before_late_probes() {
    let first = Reservation::new().address();
    let second = Reservation::new().address();
    assert!(second.port() > first.port());
}

#[test]
fn exhausted_or_fully_automatic_candidate_space_is_an_explicit_error() {
    for (mut next, automatic) in [
        (65536, (32768, 60999)),
        (1024, (0, 65535)),
        (65535, (65535, 65535)),
    ] {
        assert_eq!(
            reserve_endpoint(&mut next, automatic).unwrap_err().kind(),
            std::io::ErrorKind::AddrNotAvailable
        );
        assert_eq!(next, 65536);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn automatic_outbound_clients_cannot_claim_a_reserved_receiver_handoff() {
    use std::net::TcpStream;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    let server = listener();
    let mut reservation = Reservation::new();
    let address = reservation.address();
    let bounds = u32::from(address.port().saturating_sub(1)) | (u32::from(address.port()) << 16);
    let connect = || -> std::io::Result<TcpStream> {
        let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let descriptor = unsafe { OwnedFd::from_raw_fd(raw) };
        // Linux 6.3+ bounds only this socket's automatic choices; no namespace setting changes.
        // https://man7.org/linux/man-pages/man7/ip.7.html (IP_LOCAL_PORT_RANGE)
        if unsafe {
            libc::setsockopt(
                descriptor.as_raw_fd(),
                libc::IPPROTO_IP,
                51,
                (&bounds as *const u32).cast(),
                size_of_val(&bounds) as libc::socklen_t,
            )
        } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let endpoint = libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: server.local_addr()?.port().to_be(),
            sin_addr: libc::in_addr {
                s_addr: u32::from_ne_bytes([127, 0, 0, 1]),
            },
            sin_zero: [0; 8],
        };
        if unsafe {
            libc::connect(
                descriptor.as_raw_fd(),
                (&endpoint as *const libc::sockaddr_in).cast(),
                size_of_val(&endpoint) as libc::socklen_t,
            )
        } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(TcpStream::from(descriptor))
    };
    reservation.release_listener();
    let mut clients = Vec::new();
    for _ in 0..2 {
        match connect() {
            Ok(client) => clients.push((client, server.accept().unwrap().0)),
            Err(error) if error.raw_os_error() == Some(libc::ENOPROTOOPT) => {
                let (first, last) = automatic_port_range().unwrap();
                assert!(!(first..=last).contains(&address.port()));
                eprintln!(
                    "per-socket outbound control requires Linux 6.3; configured-range exclusion checked"
                );
                return;
            }
            Err(error) => panic!("outbound client control failed: {error}"),
        }
    }
    let receiver = TcpListener::bind(address);
    assert!(
        receiver.is_ok(),
        "automatic client claimed receiver: {}; {receiver:?}",
        diagnostics(address)
    );
}

#[test]
fn address_probe_cannot_bind_during_another_reservations_handoff() {
    use std::sync::mpsc;
    use std::time::Duration;

    let mut reservation = Reservation::new();
    let address = reservation.address();
    reservation.release_listener();
    let (entered, entering) = mpsc::channel();
    let (observed, observation) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let probe = std::thread::spawn(move || {
        entered.send(()).unwrap();
        let listener = bind(address);
        observed
            .send(listener.as_ref().map(|_| ()).map_err(|error| error.kind()))
            .unwrap();
        // A descheduled probe can retain its successful bind until after the receiver starts.
        released.recv().unwrap();
        drop(listener);
    });
    entering.recv_timeout(Duration::from_secs(2)).unwrap();
    let premature = observation.recv_timeout(Duration::from_millis(100));
    let receiver = TcpListener::bind(address);
    drop(reservation);
    release.send(()).unwrap();
    let result = match premature {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            observation.recv_timeout(Duration::from_secs(2)).unwrap()
        }
        Err(error) => panic!("probe disconnected: {error}"),
    };
    probe.join().unwrap();
    assert!(
        receiver.is_ok(),
        "probe claimed the reserved handoff endpoint {address}: {receiver:?}"
    );
    assert_eq!(result, Err(std::io::ErrorKind::AddrInUse));
}

#[test]
fn reservation_excludes_allocations_until_the_receiver_has_bound() {
    let reservation = Reservation::new();
    let address = reservation.address();
    assert_eq!(
        TcpListener::bind(address).unwrap_err().kind(),
        std::io::ErrorKind::AddrInUse
    );
    let receiver = reservation.handoff(|address| {
        assert!(
            ALLOCATION.try_lock().is_err(),
            "parallel fixture allocation entered the handoff gap"
        );
        TcpListener::bind(address).unwrap()
    });
    let another = listener();
    assert_ne!(
        receiver.local_addr().unwrap(),
        another.local_addr().unwrap()
    );
}
