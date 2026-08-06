use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use tea_coding_tools::{
    FetchAddressPolicy, FetchRequest, FetchResult, FetchUrlPolicy, FetchUrlScheme,
};

fn ip(value: &str) -> IpAddr {
    value.parse().unwrap()
}

#[test]
fn production_url_policy_canonicalizes_without_disclosing_query_in_debug() {
    let policy = FetchUrlPolicy::production();
    let url = policy
        .validate("https://Example.COM:443/a/../guide?token=secret#section")
        .unwrap();
    assert_eq!(url.as_str(), "https://example.com/guide?token=secret");
    assert_eq!(url.host(), "example.com");
    assert_eq!(url.port(), 443);
    assert_eq!(url.scheme(), FetchUrlScheme::Https);
    assert!(url.is_https());
    let debug = format!("{url:?}");
    assert!(debug.contains("has_query: true"));
    assert!(!debug.contains("secret"));

    let request = FetchRequest::new(url.as_str(), 100).unwrap();
    let debug = format!("{request:?}");
    assert!(debug.contains("example.com"));
    assert!(!debug.contains("secret"));

    let result =
        FetchResult::new(url.as_str(), url.as_str(), "text/plain", "private body").unwrap();
    let debug = format!("{result:?}");
    assert!(debug.contains("body_chars: 12"));
    assert!(!debug.contains("private body"));
    assert!(!debug.contains("secret"));
}

#[test]
fn url_policy_rejects_credentials_schemes_ports_and_downgrades() {
    let production = FetchUrlPolicy::production();
    for value in [
        "http://example.com/",
        "ftp://example.com/file",
        "https://user@example.com/",
        "https://example.com:8443/",
        "https://bad_host.example/",
        "https://example.com/has space",
    ] {
        assert!(production.validate(value).is_err(), "accepted {value}");
    }

    let source = production.validate("https://example.com/a/b").unwrap();
    assert_eq!(
        source
            .resolve_redirect("../next", &production)
            .unwrap()
            .as_str(),
        "https://example.com/next"
    );
    assert!(
        source
            .resolve_redirect("http://example.com/downgrade", &production)
            .is_err()
    );
}

#[test]
fn loopback_http_requires_an_explicit_test_policy_and_safe_port() {
    let production = FetchUrlPolicy::production();
    let fixtures = FetchUrlPolicy::loopback_tests();
    assert!(production.validate("http://127.0.0.1:8080/").is_err());
    let url = fixtures.validate("http://127.0.0.1:8080/test").unwrap();
    assert_eq!(url.port(), 8080);
    assert_eq!(url.scheme(), FetchUrlScheme::Http);
    assert!(fixtures.validate("http://localhost:22/").is_err());
    assert!(fixtures.validate("http://192.168.1.1:8080/").is_err());
}

#[test]
fn address_policy_rejects_special_ipv4_and_mixed_answers() {
    let policy = FetchAddressPolicy::public_network();
    for value in [
        "0.0.0.0",
        "10.0.0.1",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "192.0.2.1",
        "192.168.0.1",
        "198.18.0.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "255.255.255.255",
    ] {
        assert!(!policy.allows(ip(value)), "allowed {value}");
    }
    assert!(policy.allows(ip("93.184.216.34")));
    assert!(
        policy
            .validate([ip("93.184.216.34"), ip("10.0.0.1")])
            .is_err()
    );
    assert!(policy.validate([]).is_err());
}

#[test]
fn address_policy_rejects_special_ipv6_and_mapped_private_addresses() {
    let policy = FetchAddressPolicy::public_network();
    for value in [
        "::",
        "::1",
        "::ffff:10.0.0.1",
        "64:ff9b::192.0.2.1",
        "100::1",
        "2001::1",
        "2001:2::1",
        "2001:db8::1",
        "2002:c000:0201::1",
        "3fff::1",
        "fc00::1",
        "fe80::1",
        "fec0::1",
        "ff00::1",
    ] {
        assert!(!policy.allows(ip(value)), "allowed {value}");
    }
    assert!(policy.allows(ip("2606:2800:220:1:248:1893:25c8:1946")));
}

#[test]
fn validated_addresses_are_sorted_deduplicated_and_verify_the_peer() {
    let policy = FetchAddressPolicy::public_network();
    let addresses = policy
        .validate([
            ip("93.184.216.35"),
            ip("93.184.216.34"),
            ip("93.184.216.34"),
        ])
        .unwrap();
    assert_eq!(
        addresses.addresses(),
        [ip("93.184.216.34"), ip("93.184.216.35")]
    );
    assert!(addresses.contains_peer(SocketAddr::from_str("93.184.216.34:443").unwrap(), 443));
    assert!(!addresses.contains_peer(SocketAddr::from_str("93.184.216.36:443").unwrap(), 443));
    assert!(!addresses.contains_peer(SocketAddr::from_str("93.184.216.34:80").unwrap(), 443));
    let debug = format!("{addresses:?}");
    assert_eq!(debug, "ValidatedFetchAddresses { address_count: 2 }");
}

#[test]
fn loopback_addresses_require_the_explicit_fixture_policy() {
    assert!(
        FetchAddressPolicy::public_network()
            .validate([ip("127.0.0.1")])
            .is_err()
    );
    assert!(
        FetchAddressPolicy::loopback_tests()
            .validate([ip("127.0.0.1"), ip("::1")])
            .is_ok()
    );
    assert!(
        FetchAddressPolicy::loopback_tests()
            .validate([ip("127.0.0.1"), ip("10.0.0.1")])
            .is_err()
    );
}
