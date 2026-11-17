# wexport, in Rust

The Rust implementation of the same product the Python tree in the repository root builds. One
command, a numbered picker, one self-contained folder per chat that opens by double-clicking
`index.html`.

Branch scope: this crate is the rewrite proposed after the Python implementation reached v0.1.0. The
two are kept behaviourally identical and are compared on the same committed fixture.

## Build and run

```sh
cargo build --release
./target/release/wexport ~/Downloads/a            # asks which chats
./target/release/wexport ~/Downloads/a --list     # only the listing
./target/release/wexport ~/Downloads/a --chat 2   # one chat, no questions
```

Same flags, same exit codes, same help text as the Python CLI: `--key`, `--chat`, `--list`, `--yes`,
`--out`. Requirements are the same two binaries (`ffmpeg`, `ffprobe`) plus `tesseract` only when the
key is a screenshot.

## How it maps to the Python tree

| Rust | Python | Job |
|---|---|---|
| `crypt15.rs` | `backup/decrypt.py` | varint + protobuf header, AES-256-GCM, zlib/ZIP payload |
| `keys.rs` | `keys.py` | 64 hex digits, key file, screenshot via tesseract |
| `discover.rs` | `preflight.py` | find the backup anywhere in the copy, the key gate, the crypt14 chain |
| `msgstore.rs` | `db/msgstore.py` | the only place WhatsApp's table and column names live |
| `media.rs` | `media/resolve.py`, `media/derive.py` | content-addressed placement, thumbnails, posters, waveform peaks |
| `chunks.rs` | `site/chunks.py` | month chunks as classic scripts, escaping, splitting |
| `archive.rs` | `site/build.py` | one folder per chat, manifest, README, index, prune |
| `progress.rs` | bar inside `cli.py` | the one-line progress bar |
| `ui.rs` | `cli.py` | flags, the numbered picker, the run |
| `assets/` | `site/assets/` | the viewer, byte-identical copy |

No `chat.db`: the Python implementation used a SQLite intermediate to build the chunks; here the
messages are already in memory, so the intermediate would be a file written and deleted for nothing.

## Parity, measured

On the committed fixture (`tests/fixtures/synthetic/phone`), both implementations were run into
separate output directories and compared:

- **folder names identical** (`2026-12-02-alice-8c02`, `2026-12-02-family-9fd6`);
- **data chunks byte-identical** (`2026-11.js`, `2026-12.js` for both chats), including the escaping
  of `</script>` and U+2028, the frozen timestamps with their offsets, and the inline waveform peaks;
- **manifest identical** except its `built` timestamp (the moment of the build, by definition);
- **thumbnails and posters differ in bytes**, because the encoders differ (Pillow vs ffmpeg's
  libwebp/mjpeg). Same image, same dimensions, same paths — a derivative is not a checksum.

## Tests

```sh
cargo test        # 7 unit tests (key derivation against a known value, header, decrypt, chunks) +
                  # 9 integration tests against the fixture
cargo clippy --all-targets
```

The unit test that matters most pins `derive_key` to the value the reference implementation computes
for a known root key: a silent change there would make every archive wrong in a way no fixture catches.

## What is deliberately not here

Nothing the Python tree does not do: no Google Drive download, no iPhone, no password or passkey
backups, no transcoding, no server. The archive format is the contract and it did not change.
