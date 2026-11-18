//! The optional run on a real backup.
//!
//! The synthetic fixture proves the shapes we thought of; a real `msgstore.db` proves the schema
//! WhatsApp actually ships today. There is no personal data in this repository, so the folder is
//! supplied out of band:
//!
//! ```sh
//! WEXPORT_REAL_BACKUP=~/Downloads/phone-copy cargo test --test real
//! ```
//!
//! Without the variable the test returns at once — it is not a fake fixture and must not become one.

use wexport::ui;

#[test]
fn a_real_backup_lists_its_chats() {
    let Ok(folder) = std::env::var("WEXPORT_REAL_BACKUP") else {
        return; // no backup supplied: nothing to prove here
    };
    let key = std::env::var("WEXPORT_REAL_KEY").ok();
    let mut args = vec![folder, "-l".to_string()];
    if let Some(key) = key {
        args.push("-k".to_string());
        args.push(key);
    }
    let code = ui::run(&args).expect("a real backup lists without failing");
    assert_eq!(code, 0);
}
