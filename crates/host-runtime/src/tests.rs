use super::*;
use hostable_buzzer::Buzzer;
use hostable_protocol::{HostToPhone, PhoneInput, PhoneToRelay, RelayToPhone, Widget};
use hostable_relay::server::{Config, Relay};
use hostable_rules_api::Json;

async fn relay() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(Relay::new(Config::default()).serve(listener));
    format!("ws://{addr}")
}

fn buzzer_room() -> GameRoom {
    Room::new(Box::new(Json(Buzzer)) as Box<DynRules>, 5).with_name("Buzzer")
}

struct Host {
    cmds: mpsc::UnboundedSender<Command>,
    updates: mpsc::UnboundedReceiver<Update>,
    task: tokio::task::JoinHandle<Result<(), String>>,
}

fn host(base: &str, room: GameRoom, reclaim: Option<(String, String)>) -> Host {
    let (cmds, rx) = mpsc::unbounded_channel();
    let (tx, updates) = mpsc::unbounded_channel();
    let options = Options {
        relay: base.to_string(),
        pack: "buzzer".into(),
        save_to: SaveTo::Nowhere,
        max_reconnects: Some(3),
    };
    let task = tokio::spawn(run(room, reclaim, options, rx, tx));
    Host {
        cmds,
        updates,
        task,
    }
}

impl Host {
    async fn opened(&mut self) -> (String, String) {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), self.updates.recv()).await {
                Ok(Some(Update::Opened { code, host_token })) => return (code, host_token),
                Ok(Some(_)) => continue,
                other => panic!("no room: {other:?}"),
            }
        }
    }

    async fn inspect<T: Send + 'static>(
        &self,
        f: impl FnOnce(&GameRoom) -> T + Send + 'static,
    ) -> T {
        let (tx, rx) = oneshot::channel();
        self.cmds
            .send(Command::Inspect(Box::new(move |room| {
                let _ = tx.send(f(room));
            })))
            .unwrap();
        rx.await.unwrap()
    }
}

struct Phone {
    ws: Ws,
    token: String,
    ui: Option<Widget>,
}

impl Phone {
    async fn join(base: &str, code: &str, name: &str) -> Phone {
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("{base}/ws/phone"))
            .await
            .unwrap();
        let msg = PhoneToRelay::Join {
            code: code.into(),
            name: name.into(),
            color: String::new(),
        };
        ws.send(Message::Text(serde_json::to_string(&msg).unwrap().into()))
            .await
            .unwrap();
        let mut phone = Phone {
            ws,
            token: String::new(),
            ui: None,
        };
        while phone.token.is_empty() {
            if let RelayToPhone::Joined { player_token, .. } = phone.next().await {
                phone.token = player_token;
            }
        }
        phone
    }

    async fn rejoin(base: &str, code: &str, token: &str) -> Phone {
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("{base}/ws/phone"))
            .await
            .unwrap();
        let msg = PhoneToRelay::Rejoin {
            code: code.into(),
            player_token: token.into(),
        };
        ws.send(Message::Text(serde_json::to_string(&msg).unwrap().into()))
            .await
            .unwrap();
        Phone {
            ws,
            token: token.into(),
            ui: None,
        }
    }

    async fn next(&mut self) -> RelayToPhone {
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(5), self.ws.next())
                .await
                .expect("phone waited too long")
                .expect("socket closed")
                .unwrap();
            if let Message::Text(t) = msg {
                let m: RelayToPhone = serde_json::from_str(&t).unwrap();
                if let RelayToPhone::FromHost {
                    message: HostToPhone::Ui { ui, .. },
                } = &m
                {
                    self.ui = Some(ui.clone());
                }
                return m;
            }
        }
    }

    /// Wait for any screen.
    async fn lobby(&mut self) {
        while self.ui.is_none() {
            self.next().await;
        }
    }

    /// Wait for a screen with this button.
    async fn wait_for(&mut self, widget: &str) {
        while !self.ui.as_ref().is_some_and(|u| u.has_input(widget)) {
            self.next().await;
        }
    }

    /// Press a button. The screen shown so far is now stale.
    async fn press(&mut self, widget: &str) {
        self.ui = None;
        let msg = PhoneToRelay::Input(PhoneInput::press(widget));
        self.ws
            .send(Message::Text(serde_json::to_string(&msg).unwrap().into()))
            .await
            .unwrap();
    }
}

fn text_of(ui: &Widget) -> String {
    serde_json::to_string(ui).unwrap()
}

#[tokio::test]
async fn a_game_plays_over_the_relay_with_a_phone_reconnect() {
    let base = relay().await;
    let mut h = host(&base, buzzer_room(), None);
    let (code, _) = h.opened().await;
    let mut ana = Phone::join(&base, &code, "Ana").await;
    let mut bo = Phone::join(&base, &code, "Bo").await;
    ana.lobby().await;
    bo.lobby().await;
    h.cmds.send(Command::Start).unwrap();
    for round in 0..5 {
        bo.wait_for("buzz").await;
        bo.press("buzz").await;
        if round < 4 {
            ana.wait_for("next_round").await;
            if round == 2 {
                // Ana's phone drops and comes back with her token.
                let token = ana.token.clone();
                drop(ana);
                ana = Phone::rejoin(&base, &code, &token).await;
                ana.wait_for("next_round").await;
            }
            ana.press("next_round").await;
        }
    }
    loop {
        if bo
            .ui
            .as_ref()
            .is_some_and(|u| text_of(u).contains("You win!"))
        {
            break;
        }
        bo.next().await;
    }
    let finished = h.inspect(|room| room.finished().cloned()).await.unwrap();
    assert_eq!(finished.ranking[0], vec![1]);
    h.cmds.send(Command::Close).unwrap();
    assert_eq!(h.task.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn a_restarted_host_reclaims_its_room_and_phones_keep_their_seats() {
    let base = relay().await;
    let mut h = host(&base, buzzer_room(), None);
    let (code, token) = h.opened().await;
    let mut ana = Phone::join(&base, &code, "Ana").await;
    let mut bo = Phone::join(&base, &code, "Bo").await;
    ana.lobby().await;
    bo.lobby().await;
    h.cmds.send(Command::Start).unwrap();
    bo.wait_for("buzz").await;
    bo.press("buzz").await;
    ana.wait_for("next_round").await;
    let (tx, rx) = oneshot::channel();
    h.cmds.send(Command::Save(tx)).unwrap();
    let saved = rx.await.unwrap().unwrap();
    // The host process dies.
    h.task.abort();
    loop {
        if matches!(ana.next().await, RelayToPhone::HostAway) {
            break;
        }
    }
    // It comes back from its save.
    let room = saved.restore(Box::new(Json(Buzzer))).unwrap();
    let mut h2 = host(&base, room, Some((code.clone(), token)));
    assert_eq!(h2.opened().await.0, code);
    ana.ui = None;
    ana.wait_for("next_round").await;
    ana.press("next_round").await;
    bo.wait_for("buzz").await;
    let shown = text_of(bo.ui.as_ref().unwrap());
    assert!(shown.contains("Round 2 · you have 1"), "{shown}");
    let rounds = h2
        .inspect(|room| room.state().unwrap()["round"].clone())
        .await;
    assert_eq!(rounds, serde_json::json!(2));
}

#[tokio::test]
async fn a_detached_game_is_taken_over_by_another_host_with_the_same_tokens() {
    let base = relay().await;
    let mut h = host(&base, buzzer_room(), None);
    let (code, _) = h.opened().await;
    let mut ana = Phone::join(&base, &code, "Ana").await;
    let mut bo = Phone::join(&base, &code, "Bo").await;
    ana.lobby().await;
    bo.lobby().await;
    h.cmds.send(Command::Start).unwrap();
    ana.wait_for("buzz").await;
    ana.press("buzz").await;
    ana.wait_for("next_round").await;
    let (tx, rx) = oneshot::channel();
    h.cmds.send(Command::Detach(tx)).unwrap();
    let saved = rx.await.unwrap().unwrap();
    assert!(h.task.await.unwrap().is_ok());
    // A second host (the cloud) picks it up from the save.
    let room = saved.restore(Box::new(Json(Buzzer))).unwrap();
    let mut cloud = host(
        &base,
        room,
        Some((saved.code.clone(), saved.host_token.clone())),
    );
    assert_eq!(cloud.opened().await.0, code);
    // Phones never reconnected: same sockets, same seats.
    ana.ui = None;
    ana.wait_for("next_round").await;
    ana.press("next_round").await;
    bo.wait_for("buzz").await;
    assert!(text_of(bo.ui.as_ref().unwrap()).contains("you have 0"));
}

#[tokio::test]
async fn share_link_seats_are_added_through_the_relay() {
    let base = relay().await;
    let mut h = host(&base, buzzer_room(), None);
    let (code, _) = h.opened().await;
    h.cmds.send(Command::AddPlayer("Remote Ri".into())).unwrap();
    let token = loop {
        if let Some(Update::PlayerAdded { player_token, .. }) = h.updates.recv().await {
            break player_token;
        }
    };
    let mut ri = Phone::rejoin(&base, &code, &token).await;
    loop {
        if let RelayToPhone::FromHost {
            message: HostToPhone::Seat { seat, name, .. },
        } = ri.next().await
        {
            assert_eq!((seat, name.as_str()), (Some(0), "Remote Ri"));
            break;
        }
    }
}

#[tokio::test]
async fn a_game_over_its_cpu_quota_pauses() {
    let metered = Metered::new(
        Box::new(Json(Buzzer)),
        Quota {
            busy: Duration::ZERO,
            window: Duration::from_secs(60),
        },
    );
    use hostable_rules_api::{Rules, Seat};
    // Even setup is refused once the (zero) quota is used.
    let first = metered.setup(&[Seat::new(0, "A"), Seat::new(1, "B")], 1);
    assert!(
        matches!(first, Err(hostable_rules_api::RulesError::Crashed(m)) if m.contains("quota"))
    );
    let generous = Metered::new(Box::new(Json(Buzzer)), Quota::default());
    assert!(generous
        .setup(&[Seat::new(0, "A"), Seat::new(1, "B")], 1)
        .is_ok());
    assert!(generous.busy() < Duration::from_secs(1));
}
