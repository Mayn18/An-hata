use std::panic::{catch_unwind, AssertUnwindSafe};

use anahata_library::{Database, Queries};

#[test]
fn panicking_query_does_not_shrink_the_reader_pool() {
    let dir = std::env::temp_dir().join(format!("anahata_pool_{}", std::process::id()));
    let db = Database::open(dir.join("library.db")).unwrap();

    // The pool has 4 connections; losing one per panic used to deadlock on the fifth call.
    for _ in 0..8 {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let _ = db.readers.with_connection(|_| -> anahata_library::LibraryResult<()> { panic!("boom") });
        }));
        assert!(r.is_err());
    }
    assert_eq!(Queries::count_tracks(&db).unwrap(), 0);
    let _ = std::fs::remove_dir_all(dir);
}
