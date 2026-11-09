//! Network-interface change notifications (rtnetlink), so a VPN interface
//! appearing or disappearing is noticed without polling: the watcher thread
//! sleeps in `recv()` until the kernel reports a link change.
//!
//! The interface name and event type are read from the messages themselves:
//! the kernel sends RTM_DELLINK *before* the interface leaves
//! /sys/class/net, so re-reading sysfs on an event can miss a removal.

use crate::cli::is_vpn_iface;
use std::collections::BTreeSet;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

const RTM_NEWLINK: u16 = 16;
const RTM_DELLINK: u16 = 17;
const IFLA_IFNAME: u16 = 3;
const NLMSG_HDRLEN: usize = 16;
const IFINFOMSG_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    New(String),
    Del(String),
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// Link events in one netlink datagram (other message types are skipped).
/// Malformed data ends the parsing instead of panicking.
pub fn parse(buf: &[u8]) -> Vec<LinkEvent> {
    let mut out = Vec::new();
    let mut off = 0;
    while let (Some(len), Some(typ)) = (u32_at(buf, off), u16_at(buf, off + 4)) {
        let len = len as usize;
        if len < NLMSG_HDRLEN || off + len > buf.len() {
            break;
        }
        if typ == RTM_NEWLINK || typ == RTM_DELLINK {
            let mut a = off + NLMSG_HDRLEN + IFINFOMSG_LEN;
            while let (Some(alen), Some(atyp)) = (u16_at(buf, a), u16_at(buf, a + 2)) {
                let alen = alen as usize;
                if alen < 4 || a + alen > off + len {
                    break;
                }
                if atyp == IFLA_IFNAME {
                    let raw = &buf[a + 4..a + alen];
                    let name =
                        String::from_utf8_lossy(raw.split(|&c| c == 0).next().unwrap_or(raw))
                            .into_owned();
                    out.push(if typ == RTM_NEWLINK {
                        LinkEvent::New(name)
                    } else {
                        LinkEvent::Del(name)
                    });
                    break;
                }
                a += align4(alen);
            }
        }
        off += align4(len);
    }
    out
}

/// VPN interfaces currently present, updated from events.
#[derive(Debug, Default)]
pub struct VpnLinks(BTreeSet<String>);

impl VpnLinks {
    pub fn new(initial: impl IntoIterator<Item = String>) -> VpnLinks {
        VpnLinks(initial.into_iter().filter(|n| is_vpn_iface(n)).collect())
    }

    pub fn any(&self) -> bool {
        !self.0.is_empty()
    }

    /// Apply events; true if "any VPN interface present" changed.
    pub fn apply(&mut self, events: &[LinkEvent]) -> bool {
        let before = self.any();
        for e in events {
            match e {
                LinkEvent::New(n) if is_vpn_iface(n) => {
                    self.0.insert(n.clone());
                }
                LinkEvent::Del(n) => {
                    self.0.remove(n);
                }
                _ => {}
            }
        }
        self.any() != before
    }
}

/// Socket subscribed to link (interface) events.
pub fn subscribe() -> io::Result<OwnedFd> {
    // SAFETY: plain socket/bind calls; the fd is owned (and closed) by OwnedFd.
    unsafe {
        let fd = libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        );
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = OwnedFd::from_raw_fd(fd);
        let mut addr: libc::sockaddr_nl = std::mem::zeroed();
        addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        addr.nl_groups = libc::RTMGRP_LINK as u32;
        let len = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
        if libc::bind(
            fd.as_raw_fd(),
            &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
            len,
        ) < 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(fd)
    }
}

/// Watch interfaces from a dedicated thread; `on_change(any_vpn_interface)` is
/// called when a VPN interface appears while there was none, or the last one
/// disappears. Subscribe *before* reading the initial state so nothing is missed.
/// Returns an error if the kernel interface is unavailable: poll instead.
pub fn spawn(
    initial: impl FnOnce() -> Vec<String>,
    on_change: impl Fn(bool) + Send + 'static,
    on_overrun: impl Fn() + Send + 'static,
) -> io::Result<bool> {
    let fd = subscribe()?;
    let mut links = VpnLinks::new(initial());
    let present = links.any();
    std::thread::Builder::new()
        .name("netlink".into())
        .spawn(move || {
            let mut buf = vec![0u8; 32 * 1024];
            loop {
                // SAFETY: buf is valid for buf.len() bytes for the whole call.
                let n =
                    unsafe { libc::recv(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
                if n > 0 {
                    if links.apply(&parse(&buf[..n as usize])) {
                        on_change(links.any());
                    }
                    continue;
                }
                match io::Error::last_os_error().raw_os_error() {
                    Some(libc::EINTR) => continue,
                    // events were dropped (buffer overrun): resynchronise from scratch
                    Some(libc::ENOBUFS) => on_overrun(),
                    _ => break,
                }
            }
        })?;
    Ok(present)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a netlink datagram with link messages (name, type).
    fn datagram(msgs: &[(u16, &str)]) -> Vec<u8> {
        let mut b = Vec::new();
        for (typ, name) in msgs {
            let mut attr = Vec::new();
            let alen = 4 + name.len() + 1;
            attr.extend_from_slice(&(alen as u16).to_ne_bytes());
            attr.extend_from_slice(&IFLA_IFNAME.to_ne_bytes());
            attr.extend_from_slice(name.as_bytes());
            attr.push(0);
            while attr.len() % 4 != 0 {
                attr.push(0);
            }
            // an unrelated attribute first (IFLA_MTU = 4), to exercise attribute skipping
            let mut mtu = Vec::new();
            mtu.extend_from_slice(&8u16.to_ne_bytes());
            mtu.extend_from_slice(&4u16.to_ne_bytes());
            mtu.extend_from_slice(&1500u32.to_ne_bytes());
            let len = NLMSG_HDRLEN + IFINFOMSG_LEN + mtu.len() + attr.len();
            b.extend_from_slice(&(len as u32).to_ne_bytes());
            b.extend_from_slice(&typ.to_ne_bytes());
            b.extend_from_slice(&[0u8; 10]); // flags, seq, pid
            b.extend_from_slice(&[0u8; IFINFOMSG_LEN]);
            b.extend_from_slice(&mtu);
            b.extend_from_slice(&attr);
        }
        b
    }

    #[test]
    fn parses_new_and_del_link_messages() {
        let d = datagram(&[
            (RTM_NEWLINK, "tun0"),
            (RTM_DELLINK, "wg0"),
            (24 /* RTM_NEWROUTE */, "x"),
        ]);
        assert_eq!(
            parse(&d),
            [LinkEvent::New("tun0".into()), LinkEvent::Del("wg0".into())]
        );
    }

    #[test]
    fn malformed_data_does_not_panic() {
        let d = datagram(&[(RTM_NEWLINK, "tun0")]);
        for cut in 0..d.len() {
            let _ = parse(&d[..cut]);
        }
        assert!(parse(&[]).is_empty());
        assert!(parse(&[0xff; 64]).is_empty());
        let mut bad = d.clone();
        bad[0..4].copy_from_slice(&3u32.to_ne_bytes()); // length smaller than a header
        assert!(parse(&bad).is_empty());
    }

    #[test]
    fn tracks_vpn_interfaces_from_events() {
        let mut l = VpnLinks::new(["lo".to_string(), "wlp3s0".to_string()]);
        assert!(!l.any());
        // the CLI renames/recreates: several messages per change
        assert!(l.apply(&[LinkEvent::New("tun0".into()), LinkEvent::New("tun0".into())]));
        assert!(l.any());
        assert!(!l.apply(&[LinkEvent::New("tun0".into())]), "no change");
        assert!(
            !l.apply(&[
                LinkEvent::New("wlp3s0".into()),
                LinkEvent::Del("eth0".into())
            ]),
            "other interfaces"
        );
        // removal is seen from the event, even if sysfs still lists the interface
        assert!(l.apply(&[LinkEvent::Del("tun0".into())]));
        assert!(!l.any());
    }

    #[test]
    fn last_vpn_interface_counts() {
        let mut l = VpnLinks::new(["tun0".to_string(), "wg0".to_string()]);
        assert!(
            !l.apply(&[LinkEvent::Del("tun0".into())]),
            "wg0 still there"
        );
        assert!(l.apply(&[LinkEvent::Del("wg0".into())]));
    }

    #[test]
    fn subscribes_to_link_events() {
        // available to unprivileged users on Linux
        assert!(subscribe().is_ok());
    }
}
