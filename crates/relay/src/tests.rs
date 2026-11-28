use super::*;
use hostable_protocol::{HostToPhone, Widget};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

fn drain<T: serde::de::DeserializeOwned>(rx: &mut UnboundedReceiver<String>) -> Vec<T> {
    let mut out = Vec::new();
    while let Ok(text) = rx.try_recv() {
        out.push(serde_json::from_str(&text).unwrap());
    }
    out
}

fn host_token(rx: &mut UnboundedReceiver<String>) -> String {
    match drain::<RelayToHost>(rx).remove(0) {
        RelayToHost::RoomCreated { host_token, .. } => host_token,
        other => panic!("expected RoomCreated, got {other:?}"),
    }
}

#[test]
fn codes_use_only_unambiguous_letters() {
    for _ in 0..200 {
        let code = new_code();
        assert_eq!(code.len(), CODE_LEN);
        assert!(code.bytes().all(|b| CODE_LETTERS.contains(&b)));
    }
}

#[test]
fn names_and_colours_are_cleaned() {
    assert_eq!(clean_name("  Ana \n  B\u{7}o "), Some("Ana Bo".into()));
    assert_eq!(clean_name("   "), None);
    assert_eq!(clean_name(&"x".repeat(100)).unwrap().len(), MAX_NAME_CHARS);
    assert_eq!(clean_color("#A0b1C2"), "#a0b1c2");
    assert_eq!(clean_color("url(javascript:x)"), "");
}

#[test]
fn join_play_and_rejoin() {
    let mut reg = Registry::default();
    let (host_tx, mut host_rx) = unbounded_channel();
    let (code, host) = reg.create_room(host_tx);
    host_token(&mut host_rx);

    // A phone joins with a lower-case code.
    let (phone_tx, mut phone_rx) = unbounded_channel();
    let (player, conn) = reg
        .join(&code.to_lowercase(), "Ana", "#FF0000", phone_tx)
        .unwrap();
    let RelayToPhone::Joined { player_token, .. } = drain(&mut phone_rx).remove(0) else {
        panic!("expected Joined");
    };
    assert_eq!(
        drain::<RelayToHost>(&mut host_rx),
        vec![RelayToHost::PlayerJoined {
            player,
            name: "Ana".into(),
            color: "#ff0000".into()
        }]
    );

    // Host to phone and phone to host.
    let message = HostToPhone::Ui {
        version: 1,
        ui: Widget::text("hi"),
    };
    reg.from_host(
        &code,
        host,
        HostToRelay::ToPlayer {
            player,
            message: message.clone(),
        },
    );
    assert_eq!(
        drain::<RelayToPhone>(&mut phone_rx),
        vec![RelayToPhone::FromHost { message }]
    );
    reg.from_player(&code, player, conn, PhoneInput::press("buzz"));
    assert_eq!(
        drain::<RelayToHost>(&mut host_rx),
        vec![RelayToHost::FromPlayer {
            player,
            input: PhoneInput::press("buzz")
        }]
    );

    // The phone drops and comes back with its token.
    reg.player_left(&code, player, conn);
    assert_eq!(
        drain::<RelayToHost>(&mut host_rx),
        vec![RelayToHost::PlayerAway { player }]
    );
    let (phone_tx, mut phone_rx) = unbounded_channel();
    assert_eq!(
        reg.rejoin(&code, "wrong", phone_tx.clone()),
        Err(RelayError::BadToken)
    );
    let (again, conn2) = reg.rejoin(&code, &player_token, phone_tx).unwrap();
    assert_eq!(again, player);
    assert_eq!(
        drain::<RelayToPhone>(&mut phone_rx),
        vec![RelayToPhone::Rejoined { player }]
    );
    // Input from the old connection is ignored now.
    drain::<RelayToHost>(&mut host_rx);
    reg.from_player(&code, player, conn, PhoneInput::press("stale"));
    assert!(drain::<RelayToHost>(&mut host_rx).is_empty());
    reg.from_player(&code, player, conn2, PhoneInput::press("fresh"));
    assert_eq!(drain::<RelayToHost>(&mut host_rx).len(), 1);
}

#[test]
fn a_second_tab_takes_over_the_seat() {
    let mut reg = Registry::default();
    let (host_tx, mut host_rx) = unbounded_channel();
    let (code, _) = reg.create_room(host_tx);
    host_token(&mut host_rx);
    let (tx1, mut rx1) = unbounded_channel();
    let (player, c1) = reg.join(&code, "Ana", "", tx1).unwrap();
    let RelayToPhone::Joined { player_token, .. } = drain(&mut rx1).remove(0) else {
        panic!()
    };
    let (tx2, _rx2) = unbounded_channel();
    reg.rejoin(&code, &player_token, tx2).unwrap();
    assert!(matches!(
        &drain::<RelayToPhone>(&mut rx1)[..],
        [RelayToPhone::Error { .. }]
    ));
    // The old socket closing doesn't mark the seat away.
    drain::<RelayToHost>(&mut host_rx);
    reg.player_left(&code, player, c1);
    assert!(drain::<RelayToHost>(&mut host_rx).is_empty());
}

#[test]
fn hosts_can_add_seats_for_share_links() {
    let mut reg = Registry::default();
    let (host_tx, mut host_rx) = unbounded_channel();
    let (code, host) = reg.create_room(host_tx);
    host_token(&mut host_rx);
    reg.from_host(
        &code,
        host,
        HostToRelay::AddPlayer {
            name: "Remote Ri".into(),
        },
    );
    let RelayToHost::PlayerAdded {
        player,
        player_token,
        name,
    } = drain(&mut host_rx).remove(0)
    else {
        panic!("expected PlayerAdded")
    };
    assert_eq!(name, "Remote Ri");
    let (tx, mut rx) = unbounded_channel();
    assert_eq!(reg.rejoin(&code, &player_token, tx).unwrap().0, player);
    assert_eq!(
        drain::<RelayToPhone>(&mut rx),
        vec![RelayToPhone::Rejoined { player }]
    );
    assert_eq!(
        drain::<RelayToHost>(&mut host_rx),
        vec![RelayToHost::PlayerReturned { player }]
    );
}

#[test]
fn locked_rooms_refuse_new_players() {
    let mut reg = Registry::default();
    let (host_tx, _host_rx) = unbounded_channel();
    let (code, host) = reg.create_room(host_tx);
    reg.from_host(&code, host, HostToRelay::Lock { locked: true });
    let (phone_tx, _rx) = unbounded_channel();
    assert_eq!(
        reg.join(&code, "Ana", "", phone_tx),
        Err(RelayError::RoomLocked)
    );
}

#[test]
fn a_room_waits_for_its_host_then_closes() {
    let mut reg = Registry::default();
    let (host_tx, mut host_rx) = unbounded_channel();
    let (code, host) = reg.create_room(host_tx);
    let token = host_token(&mut host_rx);
    let (phone_tx, mut phone_rx) = unbounded_channel();
    reg.join(&code, "Ana", "", phone_tx).unwrap();
    drain::<RelayToPhone>(&mut phone_rx);

    let t0 = Instant::now();
    reg.host_left(&code, host, t0);
    assert_eq!(
        drain::<RelayToPhone>(&mut phone_rx),
        vec![RelayToPhone::HostAway]
    );
    reg.sweep(t0 + Duration::from_secs(60));
    let (host_tx, _host_rx) = unbounded_channel();
    let host = reg.reclaim_room(&code, &token, host_tx).unwrap();
    assert_eq!(
        drain::<RelayToPhone>(&mut phone_rx),
        vec![RelayToPhone::HostBack]
    );

    reg.host_left(&code, host, t0);
    reg.sweep(t0 + HOST_GRACE + Duration::from_secs(1));
    assert_eq!(reg.room_count(), 0);
}

#[test]
fn a_new_host_with_the_token_takes_over_and_the_old_one_is_cut_off() {
    let mut reg = Registry::default();
    let (old_tx, mut old_rx) = unbounded_channel();
    let (code, old) = reg.create_room(old_tx);
    let token = host_token(&mut old_rx);
    let (new_tx, mut new_rx) = unbounded_channel();
    let new = reg.reclaim_room(&code, &token, new_tx).unwrap();
    assert!(matches!(
        &drain::<RelayToHost>(&mut old_rx)[..],
        [RelayToHost::Error { .. }]
    ));
    assert_eq!(
        drain::<RelayToHost>(&mut new_rx),
        vec![RelayToHost::RoomReclaimed {
            code: code.clone(),
            connected: vec![]
        }]
    );
    assert!(!reg.from_host(&code, old, HostToRelay::Lock { locked: true }));
    // The old host's socket closing doesn't strand the new one.
    reg.host_left(&code, old, Instant::now());
    assert!(reg.from_host(&code, new, HostToRelay::Lock { locked: false }));
}

#[test]
fn rooms_and_tokens_survive_a_restart() {
    let mut reg = Registry::default();
    let (host_tx, mut host_rx) = unbounded_channel();
    let (code, _) = reg.create_room(host_tx);
    let token = host_token(&mut host_rx);
    let (tx, mut rx) = unbounded_channel();
    let (player, _) = reg.join(&code, "Ana", "", tx).unwrap();
    let RelayToPhone::Joined { player_token, .. } = drain(&mut rx).remove(0) else {
        panic!()
    };
    assert!(reg.is_dirty());
    let text = serde_json::to_string(&reg.save()).unwrap();

    let mut reg = Registry::restore(serde_json::from_str(&text).unwrap(), Instant::now());
    let (tx, mut rx) = unbounded_channel();
    assert_eq!(reg.rejoin(&code, &player_token, tx).unwrap().0, player);
    // No host yet, so the phone is told to wait.
    assert_eq!(
        drain::<RelayToPhone>(&mut rx),
        vec![RelayToPhone::Rejoined { player }, RelayToPhone::HostAway]
    );
    let (host_tx, _) = unbounded_channel();
    assert!(reg.reclaim_room(&code, &token, host_tx).is_ok());
    // New players get fresh ids.
    let (tx, _) = unbounded_channel();
    assert_eq!(reg.join(&code, "Bo", "", tx).unwrap().0, player + 1);
}
