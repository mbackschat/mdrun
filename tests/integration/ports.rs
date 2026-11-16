use std::net::{SocketAddr, TcpListener};
use std::sync::{Mutex, MutexGuard};

// The live socket excludes other bind(2) callers during preparation; this gate
// excludes this binary's other fixture allocations during close-to-ready handoff.
static ALLOCATION: Mutex<()> = Mutex::new(());

pub(super) struct Reservation {
    listener: Option<TcpListener>,
    address: SocketAddr,
    _allocation: MutexGuard<'static, ()>,
}

impl Reservation {
    pub(super) fn new() -> Self {
        let allocation = ALLOCATION.lock().unwrap_or_else(|error| error.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
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
