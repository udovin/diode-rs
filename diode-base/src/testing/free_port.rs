//! Free Port Utility
//!
//! This module provides a utility for managing free ports in tests to avoid port conflicts.
//! The `FreePort` struct automatically reserves and releases ports using a global registry.

use rand::Rng;
use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};
use std::sync::{LazyLock, Mutex};

/// Global registry of allocated ports to prevent conflicts between tests
static ALLOCATED_PORTS: LazyLock<Mutex<HashSet<u16>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// A wrapper around a port number that guarantees the port is free and manages its lifecycle
#[derive(Debug)]
pub struct FreePort(u16);

impl FreePort {
    /// Creates a new FreePort by finding an available port
    ///
    /// This method will attempt up to 16 times to find a free port by:
    /// 1. Generating a random port number in the range 8000-65000
    /// 2. Checking that it's not already allocated in the global registry
    /// 3. Attempting to bind to the port to verify it's actually free
    /// 4. Adding it to the global registry to prevent other threads from using it
    ///
    /// # Panics
    ///
    /// Panics if unable to find a free port after 16 attempts
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();

        for _ in 0..16 {
            // Generate random port in range 8000-65000
            let port = rng.gen_range(8000..=65000);

            // Check if port is already allocated
            {
                let allocated = ALLOCATED_PORTS.lock().unwrap();
                if allocated.contains(&port) {
                    continue;
                }
            }

            // Try to bind to the port to verify it's actually free
            if let Ok(listener) = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)) {
                // Close the listener immediately - we just wanted to check availability
                drop(listener);

                // Add to allocated ports registry
                {
                    let mut allocated = ALLOCATED_PORTS.lock().unwrap();
                    if allocated.insert(port) {
                        // Successfully inserted (wasn't already there)
                        return FreePort(port);
                    }
                    // If insert returned false, another thread beat us to it, try again
                }
            }
        }

        panic!("Unable to find a free port after 16 attempts");
    }

    /// Returns the port number
    pub fn port(&self) -> u16 {
        self.0
    }

    /// Returns the port as a formatted string for binding addresses
    pub fn as_addr(&self) -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.0))
    }
}

impl Drop for FreePort {
    /// Automatically removes the port from the global registry when dropped
    fn drop(&mut self) {
        let mut allocated = ALLOCATED_PORTS.lock().unwrap();
        allocated.remove(&self.0);
    }
}

impl Default for FreePort {
    fn default() -> Self {
        FreePort::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_free_port_allocation() {
        let port = FreePort::new();
        assert!(port.port() >= 8000 && port.port() <= 65000);

        // Verify port is in allocated registry
        {
            let allocated = ALLOCATED_PORTS.lock().unwrap();
            assert!(allocated.contains(&port.port()));
        }
    }

    #[test]
    fn test_free_port_release() {
        let port_num = {
            let port = FreePort::new();
            let port_num = port.port();

            // Verify port is allocated
            {
                let allocated = ALLOCATED_PORTS.lock().unwrap();
                assert!(allocated.contains(&port_num));
            }

            port_num
        }; // port is dropped here

        // Verify port is released
        {
            let allocated = ALLOCATED_PORTS.lock().unwrap();
            assert!(!allocated.contains(&port_num));
        }
    }

    #[test]
    fn test_multiple_ports_no_conflict() {
        let port1 = FreePort::new();
        let port2 = FreePort::new();
        let port3 = FreePort::new();

        // All ports should be different
        assert_ne!(port1.port(), port2.port());
        assert_ne!(port1.port(), port3.port());
        assert_ne!(port2.port(), port3.port());

        // All should be in allocated registry
        {
            let allocated = ALLOCATED_PORTS.lock().unwrap();
            assert!(allocated.contains(&port1.port()));
            assert!(allocated.contains(&port2.port()));
            assert!(allocated.contains(&port3.port()));
        }
    }

    #[test]
    fn test_concurrent_allocation() {
        let handles: Vec<_> = (0..10)
            .map(|_| {
                thread::spawn(|| {
                    let port = FreePort::new();
                    thread::sleep(std::time::Duration::from_millis(10));
                    port.port()
                })
            })
            .collect();

        let ports: Vec<u16> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // All ports should be unique
        let mut unique_ports = HashSet::new();
        for port in &ports {
            assert!(
                unique_ports.insert(*port),
                "Port {} was allocated twice",
                port
            );
        }

        assert_eq!(ports.len(), 10);
    }

    #[test]
    fn test_as_addr_format() {
        let port = FreePort::new();
        let expected = format!("127.0.0.1:{}", port.port());
        assert_eq!(port.as_addr().to_string(), expected);
    }
}
