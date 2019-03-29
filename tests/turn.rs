use futures::prelude::*;
use futures::sync::oneshot;

#[test]
fn turn_once() {
    let (tx, rx) = oneshot::channel();
    let rx = rx.map_err(|_| ());

    turn_me::init_with(rx);

    let _ = tx.send(());

    let polled = turn_me::turn();
    assert_eq!(polled, true);
}
