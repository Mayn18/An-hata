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
