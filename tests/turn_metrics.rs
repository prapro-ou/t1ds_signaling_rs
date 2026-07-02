use t1ds_signaling_rs::parse_turn_allocations;

/// prometheusテキストからUDP/TCPのアロケーション数を合計し、コメント行や他のメトリクスは無視する。
#[test]
fn sums_allocations_across_types_and_ignores_comments() {
    let body = "\
# HELP turn_total_allocations Total allocations
# TYPE turn_total_allocations gauge
turn_total_allocations{type=\"UDP\"} 3
turn_total_allocations{type=\"TCP\"} 2
stun_binding_request 42
";
    assert_eq!(parse_turn_allocations(body), 5);
}

/// アロケーションが1つも無ければ0を返す。
#[test]
fn returns_zero_when_no_allocations_present() {
    assert_eq!(parse_turn_allocations("stun_binding_request 1\n"), 0);
}
