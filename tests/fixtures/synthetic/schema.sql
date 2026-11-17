-- The tables wexport reads, reduced to the columns it uses. Names mirror Android WhatsApp 2.26.
CREATE TABLE jid (
    _id INTEGER PRIMARY KEY AUTOINCREMENT,
    raw_string TEXT
);

CREATE TABLE chat (
    _id INTEGER PRIMARY KEY AUTOINCREMENT,
    jid_row_id INTEGER,
    subject TEXT
);

CREATE TABLE message (
    _id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_row_id INTEGER,
    from_me INTEGER DEFAULT 0,
    key_id TEXT,
    timestamp INTEGER,
    text_data TEXT,
    status INTEGER,
    remote_resource TEXT,
    quoted_row_id INTEGER,
    edit_version INTEGER DEFAULT 0,
    latitude REAL,
    longitude REAL
);

CREATE TABLE message_media (
    message_row_id INTEGER PRIMARY KEY,
    file_path TEXT,
    mime_type TEXT,
    file_size INTEGER,
    width INTEGER,
    height INTEGER,
    media_duration INTEGER,
    media_name TEXT,
    caption TEXT
);

CREATE TABLE message_revoked (message_row_id INTEGER PRIMARY KEY);
CREATE TABLE message_system (message_row_id INTEGER PRIMARY KEY);
CREATE TABLE message_add_on (message_row_id INTEGER PRIMARY KEY, message_add_on_type INTEGER);
CREATE TABLE message_add_on_reaction (message_add_on_row_id INTEGER, reaction TEXT);
CREATE TABLE message_vcard (message_row_id INTEGER PRIMARY KEY, display_name TEXT);

CREATE TABLE call_log (
    _id INTEGER PRIMARY KEY AUTOINCREMENT,
    jid_row_id INTEGER,
    from_me INTEGER DEFAULT 0,
    timestamp INTEGER,
    duration INTEGER,
    video_call INTEGER DEFAULT 0,
    missed_call INTEGER DEFAULT 0
);

CREATE TABLE receipt_user (message_row_id INTEGER, receipt_user_jid_row_id INTEGER, played_timestamp INTEGER);
CREATE TABLE group_participant_user (group_jid_row_id INTEGER, user_jid_row_id INTEGER);
