use super::*;

const SAMPLE: &str = "\
Netid State  Recv-Q Send-Q Local Address:Port Peer Address:Port Process
tcp   LISTEN 0      128    0.0.0.0:22         0.0.0.0:*         users:((\"sshd\",pid=800,fd=3))
tcp   LISTEN 0      128    [::]:22            [::]:*           users:((\"sshd\",pid=800,fd=4))
udp   UNCONN 0      0      0.0.0.0:68         0.0.0.0:*         users:((\"dhclient\",pid=500,fd=6))
tcp   ESTAB  0      0      10.0.0.1:22        10.0.0.2:5555    users:((\"sshd\",pid=1234,fd=5)) cubic rto:204 bytes_sent:14215788 bytes_retrans:2831 bytes_acked:14212958 bytes_received:370528 segs_out:1
tcp   ESTAB  0      0      10.0.0.1:443       10.0.0.9:4444    cubic rto:210 bytes_acked:271552 bytes_received:699850
tcp   TIME-WAIT 0   0      10.0.0.1:80        10.0.0.3:6666";

#[test]
fn splits_into_listening_and_connections() {
    let s = parse_ss(SAMPLE);
    assert_eq!(s.listening.len(), 3, "LISTEN + UNCONN → listening");
    assert_eq!(s.connections.len(), 3, "ESTAB + TIME-WAIT → connections");
    assert!(s.listening.iter().any(|x| x.proto == "tcp6" && x.program == "sshd"));
    assert!(s.listening.iter().any(|x| x.proto == "udp" && x.program == "dhclient"));
}

#[test]
fn parses_program_and_traffic() {
    let s = parse_ss(SAMPLE);
    let top = &s.connections[0];
    assert_eq!(top.program, "sshd");
    assert_eq!(top.pid, Some(1234));
    assert_eq!(top.rx, Some(370528));
    assert_eq!(top.tx, Some(14215788), "bytes_sent used for the sent count");
    let acked = s.connections.iter().find(|c| c.peer == "10.0.0.9:4444").unwrap();
    assert_eq!(acked.tx, Some(271552));
    assert_eq!(acked.rx, Some(699850));
    assert!(acked.program.is_empty());
    let tw = s.connections.iter().find(|c| c.state == "TIME-WAIT").unwrap();
    assert_eq!(tw.rx, None);
    assert_eq!(tw.tx, None);
}

#[test]
fn keeps_info_tail_without_process_token() {
    let s = parse_ss(SAMPLE);
    let top = s.connections.iter().find(|c| c.program == "sshd").unwrap();
    assert!(top.info.contains("rto:204"), "ss info kept for the details view");
    assert!(!top.info.contains("users:"), "process token excluded from info");
}

#[test]
fn header_and_blank_lines_are_ignored() {
    assert_eq!(parse_ss("Netid State Recv-Q Send-Q Local Peer Proc\n\n").listening.len(), 0);
    assert_eq!(parse_ss("").connections.len(), 0);
}

#[test]
fn service_names_from_port() {
    assert_eq!(service_name(443, "tcp"), "https");
    assert_eq!(service_name(22, "tcp"), "ssh");
    assert_eq!(service_name(0, "tcp"), "");
    // A listener's local port gets the service name.
    let s = parse_ss(SAMPLE);
    let ssh = s.listening.iter().find(|l| l.local.contains(":22")).unwrap();
    assert_eq!(ssh.service, "ssh");
}

#[test]
fn loopback_detection() {
    assert!(is_loopback("127.0.0.1:80"));
    assert!(is_loopback("[::1]:631"));
    assert!(is_loopback("127.0.0.53%lo:53"));
    assert!(!is_loopback("10.0.0.1:22"));
    assert!(!is_loopback("0.0.0.0:22"));
}

#[test]
fn filter_and_toggles_narrow_the_views() {
    let mut nv = NetView::new(false, None);
    nv.apply(parse_ss(SAMPLE));

    // Text filter: "sshd" keeps its two listeners and its one attributed conn.
    nv.filter = "sshd".into();
    nv.rebuild_views();
    assert_eq!(nv.view[0].len(), 2, "two sshd listeners");
    assert_eq!(nv.view[1].len(), 1, "one sshd connection");
    nv.filter.clear();
    nv.rebuild_views();

    // Established-only drops the TIME-WAIT connection.
    nv.established_only = true;
    nv.rebuild_views();
    assert_eq!(nv.view[1].len(), 2, "two ESTAB, TIME-WAIT hidden");
    nv.established_only = false;

    // Protocol filter: UDP keeps the one udp listener, no udp connections.
    nv.proto_filter = ProtoFilter::Udp;
    nv.rebuild_views();
    assert_eq!(nv.view[0].len(), 1);
    assert_eq!(nv.view[1].len(), 0);
}

#[test]
fn sorting_orders_and_reverses() {
    let mut nv = NetView::new(false, None);
    nv.apply(parse_ss(SAMPLE));
    // Default connections sort is traffic descending → the sshd ESTAB (14 MB) first.
    let top = nv.view[1][0];
    assert_eq!(nv.connections[top].program, "sshd");
    // Sort by peer ascending.
    nv.sort[1] = NetSort::Peer;
    nv.reverse[1] = false;
    nv.rebuild_views();
    let peers: Vec<&str> = nv.view[1].iter().map(|&i| nv.connections[i].peer.as_str()).collect();
    assert!(peers.windows(2).all(|w| w[0] <= w[1]), "peers ascending: {peers:?}");
    // Reversing flips the order.
    nv.reverse[1] = true;
    nv.rebuild_views();
    let peers_r: Vec<&str> = nv.view[1].iter().map(|&i| nv.connections[i].peer.as_str()).collect();
    assert!(peers_r.windows(2).all(|w| w[0] >= w[1]), "peers descending: {peers_r:?}");
}

#[test]
fn rates_are_delta_over_time() {
    let mut nv = NetView::new(false, None);
    nv.connections = vec![Socket {
        proto: "tcp".into(),
        local: "1.1.1.1:1".into(),
        peer: "2.2.2.2:2".into(),
        rx: Some(1000),
        tx: Some(2000),
        ..Default::default()
    }];
    let key = socket_key(&nv.connections[0]);
    nv.prev.insert(key, (600, 1400)); // +400 rx, +600 tx over 2 s
    nv.compute_rates(2.0);
    assert_eq!(nv.connections[0].rx_rate, Some(200));
    assert_eq!(nv.connections[0].tx_rate, Some(300));
    assert_eq!(nv.rate_in, 200);
    assert_eq!(nv.rate_out, 300);
    // The connection's own rate history advanced by one sample (for its sparkline).
    assert_eq!(nv.rate_history.get(&socket_key(&nv.connections[0])).map(|h| h.len()), Some(1));
}

#[test]
fn renders_both_panes_without_panic() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut nv = NetView::new(false, None);
    nv.apply(parse_ss(SAMPLE));
    let theme = crate::ui::theme::Theme::mc();
    let mut t = Terminal::new(TestBackend::new(120, 32)).unwrap();
    t.draw(|f| render::render(f, f.area(), &mut nv, &theme, None)).unwrap();
    let b = t.backend().buffer();
    let mut s = String::new();
    for y in 0..b.area.height {
        for x in 0..b.area.width {
            s.push_str(b[(x, y)].symbol());
        }
    }
    assert!(s.contains("Network Connections"), "title");
    assert!(s.contains("user mode"), "limited-visibility banner in user mode");
    assert!(s.contains("Listening ports"), "listening pane");
    assert!(s.contains("Connections"), "connections pane");
    assert!(s.contains("sshd"), "a program name is shown");
    assert!(s.contains("ssh"), "a service name is shown");
}

#[test]
fn details_popup_opens_and_closes() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    let mut nv = NetView::new(false, None);
    nv.apply(parse_ss(SAMPLE));
    nv.focus = Pane::Connections;
    nv.cursor[1] = 0;
    assert!(nv.detail.is_none());
    nv.handle_key(key(KeyCode::Enter)); // open details for the selected connection
    assert!(nv.detail.is_some(), "Enter opens the details popup");
    nv.handle_key(key(KeyCode::Esc)); // any key dismisses it
    assert!(nv.detail.is_none(), "Esc closes the details popup");
}

