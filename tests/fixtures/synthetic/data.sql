-- Deterministic fixture data: one message per kind, a group and a one-to-one chat.
-- Times are built from ISO strings so they cannot drift.
INSERT INTO jid (_id, raw_string) VALUES
    (1, '393331234567@s.whatsapp.net'),
    (2, '393339999999@s.whatsapp.net'),
    (3, '120363000000000001@g.us');

INSERT INTO chat (_id, jid_row_id, subject) VALUES
    (1, 1, NULL),
    (2, 3, 'Family');

INSERT INTO message (_id, chat_row_id, from_me, key_id, timestamp, text_data, status, remote_resource, quoted_row_id, edit_version)
VALUES
    (1, 1, 0, 'k1', CAST((julianday('2026-11-17T13:32:05Z') - 2440587.5) * 86400000 AS INTEGER),
        'Hello, first message.  </script><script>alert(1)</script>', 13, NULL, NULL, 0),
    (2, 1, 1, 'k2', CAST((julianday('2026-11-17T13:33:00Z') - 2440587.5) * 86400000 AS INTEGER),
        'Line with separators \u2028 and emoji 👍', 13, NULL, 1, 0),
    (3, 1, 0, 'k3', CAST((julianday('2026-11-17T14:00:00Z') - 2440587.5) * 86400000 AS INTEGER),
        'Original text', 13, NULL, NULL, 1),
    (4, 1, 0, 'k4', CAST((julianday('2026-11-17T14:05:00Z') - 2440587.5) * 86400000 AS INTEGER),
        NULL, 0, NULL, NULL, 0),
    (5, 2, 0, 'k5', CAST((julianday('2026-12-01T09:00:00Z') - 2440587.5) * 86400000 AS INTEGER),
        NULL, 13, '393339999999@s.whatsapp.net', NULL, 0),
    (6, 2, 0, 'k6', CAST((julianday('2026-12-01T09:01:00Z') - 2440587.5) * 86400000 AS INTEGER),
        'photo caption', 13, '393339999999@s.whatsapp.net', NULL, 0),
    (7, 2, 1, 'k7', CAST((julianday('2026-12-01T09:02:00Z') - 2440587.5) * 86400000 AS INTEGER),
        'voice note', 13, NULL, NULL, 0),
    (8, 2, 1, 'k8', CAST((julianday('2026-12-01T09:03:00Z') - 2440587.5) * 86400000 AS INTEGER),
        'clip', 13, NULL, NULL, 0),
    (9, 2, 1, 'k9', CAST((julianday('2026-12-01T09:04:00Z') - 2440587.5) * 86400000 AS INTEGER),
        'document', 13, NULL, NULL, 0),
    (10, 2, 0, 'k10', CAST((julianday('2026-12-01T09:05:00Z') - 2440587.5) * 86400000 AS INTEGER),
        NULL, 13, '393339999999@s.whatsapp.net', NULL, 0),
    (11, 2, 0, 'k11', CAST((julianday('2026-12-01T09:06:00Z') - 2440587.5) * 86400000 AS INTEGER),
        'gif!', 13, '393339999999@s.whatsapp.net', NULL, 0),
    (12, 1, 1, 'k12', CAST((julianday('2026-12-02T10:00:00Z') - 2440587.5) * 86400000 AS INTEGER),
        NULL, 13, NULL, NULL, 0),
    (13, 1, 0, 'k13', CAST((julianday('2026-12-02T10:01:00Z') - 2440587.5) * 86400000 AS INTEGER),
        'Where are we meeting?', 13, NULL, NULL, 0),
    (14, 1, 1, 'k14', CAST((julianday('2026-12-02T10:02:00Z') - 2440587.5) * 86400000 AS INTEGER),
        'this message was deleted', 0, NULL, NULL, 0),
    (15, 2, 1, 'k15', CAST((julianday('2026-12-02T10:03:00Z') - 2440587.5) * 86400000 AS INTEGER),
        NULL, 13, NULL, NULL, 0),
    (16, 2, 0, 'k16', CAST((julianday('2026-12-02T10:04:00Z') - 2440587.5) * 86400000 AS INTEGER),
        'card', 13, '393339999999@s.whatsapp.net', NULL, 0);

-- image, audio, video, document, sticker, gif (one of them points at a file that is not there)
INSERT INTO message_media (message_row_id, file_path, mime_type, file_size, width, height, media_duration, media_name, caption)
VALUES
    (6, 'Media/WhatsApp Images/IMG-20261201-WA0001.png', 'image/png', 96, 1, 1, NULL, 'IMG-20261201-WA0001.png', 'photo caption'),
    (7, 'Media/WhatsApp Voice Notes/PTT-20261201-WA0002.opus', 'audio/ogg', 3000, NULL, NULL, 1200, 'PTT-20261201-WA0002.opus', NULL),
    (8, 'Media/WhatsApp Video/VID-20261201-WA0003.mp4', 'video/mp4', 2000, 16, 16, 1000, 'VID-20261201-WA0003.mp4', 'clip'),
    (9, 'Media/WhatsApp Documents/notes.txt', 'text/plain', 24, NULL, NULL, NULL, 'notes.txt', 'document'),
    (10, 'Media/WhatsApp Stickers/sticker.webp', 'image/webp', 200, NULL, NULL, NULL, 'sticker.webp', NULL),
    (11, 'Media/WhatsApp Animated Gifs/GIF-20261201-WA0004.mp4', 'video/mp4', 2000, 16, 16, 1000, 'GIF-20261201-WA0004.mp4', 'gif!'),
    (12, 'Media/WhatsApp Images/IMG-deleted-from-phone.png', 'image/png', 100, 1, 1, NULL, 'IMG-deleted-from-phone.png', NULL);

INSERT INTO message_revoked (message_row_id) VALUES (14);
INSERT INTO message_system (message_row_id) VALUES (15);
INSERT INTO message_add_on (message_row_id, message_add_on_type) VALUES (16, 1);
INSERT INTO message_add_on_reaction (message_add_on_row_id, reaction) VALUES (16, '👍');
INSERT INTO message_vcard (message_row_id, display_name) VALUES (13, 'Bob');
INSERT INTO call_log (_id, jid_row_id, from_me, timestamp, duration, video_call, missed_call) VALUES
    (1, 1, 0, CAST((julianday('2026-12-02T11:00:00Z') - 2440587.5) * 86400000 AS INTEGER), 332, 1, 0);
