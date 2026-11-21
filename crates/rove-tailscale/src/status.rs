use std::collections::BTreeSet;
use std::net::IpAddr;

use serde_json::{Map, Value};

use crate::{TailnetPeer, TailnetSnapshot, TailnetState, TailscaleError, TailscalePeerId};

fn optional_text(
    object: &Map<String, Value>,
    name: &str,
    limit: usize,
) -> Result<String, TailscaleError> {
    let value = match object.get(name) {
        None | Some(Value::Null) => return Ok(String::new()),
        Some(Value::String(value)) => value,
        _ => {
            return Err(TailscaleError::InvalidStatus(
                "a peer text field is not a string",
            ));
        }
    };
    if value.len() > limit || value.chars().any(char::is_control) {
        return Err(TailscaleError::InvalidStatus(
            "a peer text field is invalid",
        ));
    }
    Ok(value.clone())
}

fn peer(value: &Value) -> Result<TailnetPeer, TailscaleError> {
    let object = value
        .as_object()
        .ok_or(TailscaleError::InvalidStatus("a peer is not an object"))?;
    let id = TailscalePeerId::new(optional_text(object, "ID", 256)?)?;
    let values = object
        .get("TailscaleIPs")
        .and_then(Value::as_array)
        .ok_or(TailscaleError::InvalidStatus("a peer has no address list"))?;
    let mut addresses = BTreeSet::new();
    for value in values {
        let address: IpAddr = value
            .as_str()
            .and_then(|value| value.parse().ok())
            .ok_or(TailscaleError::InvalidStatus("a peer address is invalid"))?;
        if address.is_unspecified() || address.is_loopback() || address.is_multicast() {
            return Err(TailscaleError::InvalidStatus(
                "a peer address is not a device address",
            ));
        }
        addresses.insert(address);
    }
    if addresses.is_empty() {
        return Err(TailscaleError::InvalidStatus(
            "a peer has no device address",
        ));
    }
    let addresses: Vec<_> = addresses.into_iter().collect();
    let dns_name = optional_text(object, "DNSName", 1024)?
        .trim_end_matches('.')
        .to_owned();
    let hostname = optional_text(object, "HostName", 1024)?;
    let name = match (hostname.is_empty(), dns_name.is_empty()) {
        (false, _) => hostname,
        (true, false) => dns_name.clone(),
        (true, true) => addresses[0].to_string(),
    };
    let os = optional_text(object, "OS", 128)?;
    let online = match object.get("Online") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        _ => {
            return Err(TailscaleError::InvalidStatus(
                "a peer online flag is not a boolean",
            ));
        }
    };
    Ok(TailnetPeer {
        id,
        name,
        dns_name,
        addresses,
        os: if os.is_empty() { None } else { Some(os) },
        online,
    })
}

pub(crate) fn parse(bytes: &[u8], captured_at: i64) -> Result<TailnetSnapshot, TailscaleError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| TailscaleError::InvalidJson)?;
    let object = value
        .as_object()
        .ok_or(TailscaleError::InvalidStatus("status is not an object"))?;
    let state = match object.get("BackendState").and_then(Value::as_str) {
        Some("NoState") => TailnetState::NoState,
        Some("NeedsLogin") => TailnetState::NeedsLogin,
        Some("NeedsMachineAuth") => TailnetState::NeedsMachineAuth,
        Some("Stopped") => TailnetState::Stopped,
        Some("Starting") => TailnetState::Starting,
        Some("Running") => TailnetState::Running,
        Some(_) => TailnetState::Unknown,
        None => {
            return Err(TailscaleError::InvalidStatus(
                "BackendState is missing or invalid",
            ));
        }
    };
    let mut peers = Vec::new();
    let mut ids = BTreeSet::new();
    match object.get("Peer") {
        None | Some(Value::Null) => {}
        Some(Value::Object(values)) => {
            for value in values.values() {
                let peer = peer(value)?;
                if !ids.insert(peer.id.clone()) {
                    return Err(TailscaleError::InvalidStatus("duplicate peer identity"));
                }
                peers.push(peer);
            }
        }
        Some(_) => return Err(TailscaleError::InvalidStatus("Peer is not an object")),
    }
    peers.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(TailnetSnapshot {
        state,
        peers,
        captured_at,
        message: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn device(id: &str) -> Value {
        json!({"ID":id,"HostName":"server","DNSName":"server.example.ts.net.",
            "TailscaleIPs":["fd7a:115c:a1e0::2","100.64.0.2","100.64.0.2"],"Online":true})
    }

    #[test]
    fn states_and_unknown_fields_are_read_without_exposing_accounts_or_login_urls() {
        for (raw, expected) in [
            ("NoState", TailnetState::NoState),
            ("NeedsLogin", TailnetState::NeedsLogin),
            ("NeedsMachineAuth", TailnetState::NeedsMachineAuth),
            ("Stopped", TailnetState::Stopped),
            ("Starting", TailnetState::Starting),
            ("Running", TailnetState::Running),
            ("FutureState", TailnetState::Unknown),
        ] {
            let input = json!({"BackendState":raw,"AuthURL":"https://example.invalid/login", "User":{"account":"hidden"}, "Future":true});
            let snapshot = parse(&serde_json::to_vec(&input).unwrap(), 123).unwrap();
            assert_eq!(snapshot.state, expected);
            assert_eq!(snapshot.captured_at, 123);
            assert_eq!(snapshot.message, None);
            assert!(!serde_json::to_string(&snapshot).unwrap().contains("hidden"));
            assert!(
                !serde_json::to_string(&snapshot)
                    .unwrap()
                    .contains("https://")
            );
        }
    }

    #[test]
    fn stable_ids_are_independent_of_public_keys_and_names_need_not_be_unique() {
        let input = json!({"BackendState":"Running","Peer":{"key-a":device("node-a"),"key-b":device("node-b")}});
        let first = parse(&serde_json::to_vec(&input).unwrap(), 0).unwrap();
        assert_eq!(first.peers.len(), 2);
        assert_eq!(first.peers[0].id.as_str(), "node-a");
        assert_eq!(first.peers[0].dns_name, "server.example.ts.net");
        assert_eq!(
            first.peers[0].addresses[0],
            "100.64.0.2".parse::<IpAddr>().unwrap()
        );
        assert_eq!(first.peers[0].addresses.len(), 2);
        let changed = json!({"BackendState":"Running","Peer":{"new-key":device("node-a"),"key-b":device("node-b")}});
        assert_eq!(
            first.peers,
            parse(&serde_json::to_vec(&changed).unwrap(), 0)
                .unwrap()
                .peers
        );
    }

    #[test]
    fn optional_display_fields_have_bounded_fallbacks() {
        let input = json!({"BackendState":"Running","Peer":{"key":{"ID":"node","TailscaleIPs":["100.64.0.3"]}}});
        let snapshot = parse(&serde_json::to_vec(&input).unwrap(), 0).unwrap();
        assert_eq!(snapshot.peers[0].name, "100.64.0.3");
        assert!(!snapshot.peers[0].online);
        assert_eq!(snapshot.peers[0].os, None);
    }

    #[test]
    fn invalid_required_fields_do_not_become_usable_devices() {
        for invalid in [
            json!({"TailscaleIPs":["100.64.0.2"]}),
            json!({"ID":"bad\nnode","TailscaleIPs":["100.64.0.2"]}),
            json!({"ID":"node","TailscaleIPs":[]}),
            json!({"ID":"node","TailscaleIPs":["127.0.0.1"]}),
            json!({"ID":"node","TailscaleIPs":["bad"]}),
            json!({"ID":"node","TailscaleIPs":["100.64.0.2"],"Online":"yes"}),
        ] {
            let input = json!({"BackendState":"Running","Peer":{"key":invalid}});
            assert!(parse(&serde_json::to_vec(&input).unwrap(), 0).is_err());
        }
        assert!(matches!(
            parse(b"not json", 0),
            Err(TailscaleError::InvalidJson)
        ));
        assert!(parse(b"{}", 0).is_err());
        assert!(parse(br#"{"BackendState":"Running","Peer":[]}"#, 0).is_err());
    }

    #[test]
    fn duplicate_stable_ids_are_rejected() {
        let input =
            json!({"BackendState":"Running","Peer":{"a":device("node"),"b":device("node")}});
        assert!(parse(&serde_json::to_vec(&input).unwrap(), 0).is_err());
    }
}
