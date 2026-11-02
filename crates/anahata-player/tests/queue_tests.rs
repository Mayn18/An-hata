use std::path::PathBuf;
use anahata_player::{Queue, QueueItem, RepeatMode};

fn make_item(id: u64, title: &str) -> QueueItem {
    QueueItem {
        id,
        track_id: id as i64,
        path: PathBuf::from(format!("/music/{title}.mp3")),
        title: title.to_string(),
        artist: "Test Artist".to_string(),
    }
}

#[test]
fn test_queue_sequential_navigation() {
    let mut q = Queue::new();
    let items = vec![make_item(1, "Track A"), make_item(2, "Track B"), make_item(3, "Track C")];
    q.load(items, Some(0));

    assert_eq!(q.current_item().unwrap().title, "Track A");
    assert_eq!(q.next().unwrap().title, "Track B");
    assert_eq!(q.next().unwrap().title, "Track C");
    assert!(q.next().is_none(), "End of queue should return None when repeat is Off");
}

#[test]
fn test_queue_repeat_modes() {
    let mut q = Queue::new();
    let items = vec![make_item(1, "Track 1"), make_item(2, "Track 2")];
    q.load(items, Some(1));

    // Repeat All
    q.set_repeat(RepeatMode::All);
    assert_eq!(q.next().unwrap().title, "Track 1", "Next after last track with Repeat All should loop to first");

    // Repeat One
    q.set_repeat(RepeatMode::One);
    assert_eq!(q.next().unwrap().title, "Track 1", "Next with Repeat One should repeat current track");
}

#[test]
fn test_queue_shuffle_keeps_current_first() {
    let mut q = Queue::new();
    let items = vec![
        make_item(1, "Track 1"),
        make_item(2, "Track 2"),
        make_item(3, "Track 3"),
        make_item(4, "Track 4"),
        make_item(5, "Track 5"),
    ];
    q.load(items, Some(2)); // Currently playing Track 3 (index 2)

    assert_eq!(q.current_item().unwrap().title, "Track 3");

    // Enable shuffle
    q.set_shuffle(true);
    assert!(q.is_shuffle());
    assert_eq!(q.current_item().unwrap().title, "Track 3", "Current item must stay current at index 0 after shuffle");

    // Disable shuffle
    q.set_shuffle(false);
    assert!(!q.is_shuffle());
    assert_eq!(q.current_item().unwrap().title, "Track 3", "Current item must remain active when un-shuffling");
}

#[test]
fn test_remove_before_current_keeps_same_track() {
    let mut q = Queue::new();
    q.load(vec![make_item(1, "A"), make_item(2, "B"), make_item(3, "C")], Some(2));
    let out = q.remove_item(1);
    assert!(out.removed && !out.removed_current);
    assert_eq!(q.current_item().unwrap().title, "C");
}

#[test]
fn test_remove_after_current_leaves_current_alone() {
    let mut q = Queue::new();
    q.load(vec![make_item(1, "A"), make_item(2, "B"), make_item(3, "C")], Some(0));
    q.remove_item(3);
    assert_eq!(q.current_item().unwrap().title, "A");
    assert_eq!(q.len(), 2);
}

#[test]
fn test_remove_current_points_at_following_item() {
    let mut q = Queue::new();
    q.load(vec![make_item(1, "A"), make_item(2, "B"), make_item(3, "C")], Some(1));
    let out = q.remove_item(2);
    assert!(out.removed_current);
    assert_eq!(q.current_item().unwrap().title, "C");

    let out = q.remove_item(3);
    assert!(out.removed_current);
    assert!(q.current_item().is_none(), "removing the last current item leaves nothing current");
}

#[test]
fn test_remove_unknown_id_is_a_noop() {
    let mut q = Queue::new();
    q.load(vec![make_item(1, "A")], Some(0));
    let rev = q.revision();
    assert!(!q.remove_item(99).removed);
    assert_eq!(q.revision(), rev);
}

#[test]
fn test_remove_preserves_shuffle_order_of_remaining() {
    let mut q = Queue::new();
    q.load((1..=8).map(|i| make_item(i, &format!("T{i}"))).collect(), Some(0));
    q.set_shuffle(true);
    // Walk the full shuffled order.
    let mut order = vec![q.current_item().unwrap().id];
    while let Some(it) = q.skip_next() {
        order.push(it.id);
        if order.len() == 8 {
            break;
        }
    }
    // Rewind to the start of the order, then remove one item from the middle.
    q.play_item(order[0]);
    let victim = order[4];
    q.remove_item(victim);
    let mut after = vec![q.current_item().unwrap().id];
    while let Some(it) = q.skip_next() {
        after.push(it.id);
    }
    let expected: Vec<u64> = order.into_iter().filter(|&id| id != victim).collect();
    assert_eq!(after, expected, "removal must not reshuffle");
}

#[test]
fn test_skip_next_ignores_repeat_one_but_next_honours_it() {
    let mut q = Queue::new();
    q.load(vec![make_item(1, "A"), make_item(2, "B")], Some(0));
    q.set_repeat(RepeatMode::One);
    assert_eq!(q.next().unwrap().title, "A");
    assert_eq!(q.skip_next().unwrap().title, "B");
    assert_eq!(q.skip_next().unwrap().title, "A", "wraps like repeat-all");
}

#[test]
fn test_step_forward_never_wraps() {
    let mut q = Queue::new();
    q.load(vec![make_item(1, "A"), make_item(2, "B")], Some(1));
    q.set_repeat(RepeatMode::All);
    assert!(q.step_forward().is_none());
    assert_eq!(q.current_item().unwrap().title, "B", "failed step must not move");
}
