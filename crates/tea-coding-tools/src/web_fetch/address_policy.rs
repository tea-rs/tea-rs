use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use thiserror::Error;

/// Address-policy failure detected before creating an HTTP connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FetchAddressPolicyError {
    /// DNS returned no usable A or AAAA records.
    #[error("web fetch DNS response is empty")]
    Empty,
    /// At least one DNS answer belongs to a forbidden address range.
    #[error("web fetch DNS response contains a forbidden address")]
    Forbidden,
}

/// Immutable IP-address policy applied to every DNS answer and peer address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FetchAddressPolicy {
    allow_loopback: bool,
}

impl FetchAddressPolicy {
    /// Creates the production public-network policy.
    #[must_use]
    pub const fn public_network() -> Self {
        Self {
            allow_loopback: false,
        }
    }

    /// Creates a policy for deterministic loopback fixture servers.
    ///
    /// All non-loopback private, link-local, reserved, and multicast ranges
    /// remain forbidden.
    #[must_use]
    pub const fn loopback_tests() -> Self {
        Self {
            allow_loopback: true,
        }
    }

    /// Validates every resolved A/AAAA answer and returns a deterministic set.
    ///
    /// # Errors
    ///
    /// Rejects empty answers and the entire answer set when any address is
    /// forbidden. This prevents a safe first record from hiding a private or
    /// metadata address later in the DNS response.
    pub fn validate<I>(
        &self,
        addresses: I,
    ) -> Result<ValidatedFetchAddresses, FetchAddressPolicyError>
    where
        I: IntoIterator<Item = IpAddr>,
    {
        let mut addresses = addresses.into_iter().collect::<Vec<_>>();
        addresses.sort();
        addresses.dedup();
        if addresses.is_empty() {
            return Err(FetchAddressPolicyError::Empty);
        }
        if addresses
            .iter()
            .copied()
            .any(|address| !self.allows(address))
        {
            return Err(FetchAddressPolicyError::Forbidden);
        }
        Ok(ValidatedFetchAddresses { addresses })
    }

    /// Returns whether one resolved or connected address satisfies the policy.
    #[must_use]
    pub fn allows(&self, address: IpAddr) -> bool {
        match address {
            IpAddr::V4(address) => self.allows_v4(address),
            IpAddr::V6(address) => self.allows_v6(address),
        }
    }

    fn allows_v4(self, address: Ipv4Addr) -> bool {
        if address.is_loopback() {
            return self.allow_loopback;
        }
        let value = u32::from(address);
        !IPV4_FORBIDDEN
            .iter()
            .copied()
            .any(|(network, prefix)| in_v4_network(value, u32::from(network), prefix))
    }

    fn allows_v6(self, address: Ipv6Addr) -> bool {
        if address.is_loopback() {
            return self.allow_loopback;
        }
        if let Some(mapped) = address.to_ipv4_mapped() {
            return self.allows_v4(mapped);
        }
        let value = u128::from(address);
        !IPV6_FORBIDDEN
            .iter()
            .copied()
            .any(|(network, prefix)| in_v6_network(value, u128::from(network), prefix))
    }
}

impl Default for FetchAddressPolicy {
    fn default() -> Self {
        Self::public_network()
    }
}

/// Deterministic, deduplicated set of fully validated DNS answers.
#[derive(Clone, PartialEq, Eq)]
pub struct ValidatedFetchAddresses {
    addresses: Vec<IpAddr>,
}

impl ValidatedFetchAddresses {
    /// Returns validated addresses in deterministic order.
    #[must_use]
    pub fn addresses(&self) -> &[IpAddr] {
        &self.addresses
    }

    /// Returns socket addresses for one validated destination port.
    #[must_use]
    pub fn socket_addresses(&self, port: u16) -> Vec<SocketAddr> {
        self.addresses
            .iter()
            .copied()
            .map(|address| SocketAddr::new(address, port))
            .collect()
    }

    /// Verifies a connected peer against both policy and the pinned answer set.
    #[must_use]
    pub fn contains_peer(&self, peer: SocketAddr, expected_port: u16) -> bool {
        peer.port() == expected_port && self.addresses.binary_search(&peer.ip()).is_ok()
    }
}

impl fmt::Debug for ValidatedFetchAddresses {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedFetchAddresses")
            .field("address_count", &self.addresses.len())
            .finish()
    }
}

const IPV4_FORBIDDEN: &[(Ipv4Addr, u32)] = &[
    (Ipv4Addr::UNSPECIFIED, 8),
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 0, 0, 0), 24),
    (Ipv4Addr::new(192, 0, 2, 0), 24),
    (Ipv4Addr::new(192, 31, 196, 0), 24),
    (Ipv4Addr::new(192, 52, 193, 0), 24),
    (Ipv4Addr::new(192, 88, 99, 0), 24),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(192, 175, 48, 0), 24),
    (Ipv4Addr::new(198, 18, 0, 0), 15),
    (Ipv4Addr::new(198, 51, 100, 0), 24),
    (Ipv4Addr::new(203, 0, 113, 0), 24),
    (Ipv4Addr::new(224, 0, 0, 0), 4),
    (Ipv4Addr::new(240, 0, 0, 0), 4),
];

const IPV6_FORBIDDEN: &[(Ipv6Addr, u32)] = &[
    (Ipv6Addr::UNSPECIFIED, 96),
    (Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, 0, 0), 96),
    (Ipv6Addr::new(0x64, 0xff9b, 1, 0, 0, 0, 0, 0), 48),
    (Ipv6Addr::new(0x100, 0, 0, 0, 0, 0, 0, 0), 64),
    (Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x2001, 2, 0, 0, 0, 0, 0, 0), 48),
    (Ipv6Addr::new(0x2001, 0x10, 0, 0, 0, 0, 0, 0), 28),
    (Ipv6Addr::new(0x2001, 0x20, 0, 0, 0, 0, 0, 0), 28),
    (Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x2002, 0, 0, 0, 0, 0, 0, 0), 16),
    (Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20),
    (Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0), 7),
    (Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0), 10),
    (Ipv6Addr::new(0xfec0, 0, 0, 0, 0, 0, 0, 0), 10),
    (Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0), 8),
];

fn in_v4_network(value: u32, network: u32, prefix: u32) -> bool {
    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
    value & mask == network & mask
}

fn in_v6_network(value: u128, network: u128, prefix: u32) -> bool {
    let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
    value & mask == network & mask
}
