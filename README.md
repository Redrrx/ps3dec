# PS3 Decryptor

PS3Dec is a remake of the original PS3 decryptor which decrypts PS3s redump ISOs.

the original one was written in C around 11 years ago, the sole reason I rewrote this one is for learning Rust and making my own slightly faster version to add features later anytime I want.

also I love ps3.


## How does it work ?

According to [PSDev Wiki](https://www.psdevwiki.com/ps3/Bluray_disc)
a Blu-ray disc consists of sectors with a length of
2048 bytes.

Encryption:

- Some regions are encrypted some are not
- Usually even numbered regions are encrypted and odd numbered regions are not encrypted
- the encryption used is AES-128 in CBC mode with no padding

What is simply done is using a decryption key and decrypting what needs to be decrypted as for the rest it is directly
written to disk without
keeping the data in memory.


## Demo

Decrypting MX vs. ATV Untamed (USA) in less than 2 seconds on a fast enough rig! sometimes increasing the thread count too high might add a slight overhead for the dec process to start.



Please bear in mind this demonstration is done on some very idealistic conditions with a very good CPU and a good SSD.



https://github.com/user-attachments/assets/978c1827-d788-449a-a52f-6743e94cb4db



## Usage

### command line flags

| Option | What it does | Note |
|--------|--------------|------|
| `<ISO>...` | ISO files to decrypt; drag and drop works too | One ISO uses the normal CLI. Multiple ISOs open the TUI in a terminal. Leave this out to watch the current directory. |
| `-k`, `--dk <KEY>` | Use your own decryption key | 32 hex characters. Single ISO only; don't combine with `--auto`. |
| `-t`, `--tc <COUNT>` | How many CPU threads to use | Defaults to your machine's CPU thread count. More isn't always faster. |
| `-a`, `--auto` | Find the key using the ISO name | Needs a matching key in **keys/**. Already enabled for batches and directory watching. |
| `-s`, `--skip` | Exit when done instead of waiting for Enter | Paused TUI jobs and directory watching keep the app open. |
| `-o`, `--output-dir <DIR>` | Where to save decrypted ISOs | By default, outputs go beside their input ISOs. |
| `-n`, `--output-name <NAME>` | Choose the output filename | Single ISO only. Leave out the `.iso` extension. |
| `--chunk-size <MiB>` | How much of an ISO to process at once | Default: 16 MiB. Bigger chunks use more RAM; see [performance and tuning](#performance-and-tuning). |
| `--jobs <COUNT>` | How many ISOs to decrypt at once | Default: 1. Can't be combined with `--sequential`. |
| `--sequential` | Decrypt ISOs one after another | Can't be combined with `--jobs`. |
| `-h`, `--help` | Show the command line options | |
| `-V`, `--version` | Show the version | |



```
ps3dec.exe game.iso --dk yourdecryptionkey --tc 64 --chunk-size 16
```

If you don't want to keep typing your key, use [--auto](#command-line-flags). Put your `.dkey` files in **keys/**; you can get them from [Aldostools dkeys](https://ps3.aldostools.org/dkey.html). The key needs to be 32 hex characters.

```
ps3dec.exe game.iso --auto --tc 64
```

### multiple ISOs and recovery

Drop several ISOs onto the executable, or run:

```sh
ps3dec game1.iso game2.iso --jobs 2 --skip
```

The TUI shows each ISO's progress, speed, time left, and logs. ISOs you pass on the command line start automatically. Use [--jobs](#command-line-flags) to run more than one at once, and [--skip](#command-line-flags) if you want it to close when done. Otherwise it stays open.

You can also run it without any ISO paths:

```sh
ps3dec
```

This watches the directory you ran it from then use the [TUI shortcuts](#tui-shortcuts) to start decrypting them.

If you run one ISO, or send the output to a file or pipe, it keeps the normal CLI view but if one ISO fails the others can still finish, but the app reports an error when you close it.

#### TUI shortcuts

| Key | What it does | Note |
|-----|--------------|------|
| ↑ / ↓ | Select an ISO | |
| `s` | Start or resume the selected ISO | Also restarts a stopped or failed ISO. Doesn't change its place in the queue. |
| `p` | Pause or resume the selected ISO | Finishes writing the current chunk first, then lets another queued ISO run. Waiting or queued ISOs pause right away. |
| `c` | Stop the selected ISO | Keeps unfinished output so you can resume with `s`. |
| `+` or `=` | Move the selected ISO up the queue | `=` works without Shift. |
| `-` | Move the selected ISO down the queue | |
| `q`, `Esc`, or `Ctrl+C` | Close the TUI | Running ISOs finish writing their current chunks before stopping. |
| `Enter` | Close when nothing is running or queued | |


#### recovery

If you stop halfway do not delete the `.part` and `.resume` files. run the same command you ran again, or use the [TUI shortcuts](#tui-shortcuts), and it resumes what wasn't finished last session.

Recovery needs the same ISO, key, and output path any changed files or broken recovery data are unusable, finished isos keep their `.resume` file so later runs can skip them, recovery files without their source ISO aren't picked up on their own.

### performance and tuning

On an SSD, you can try `--chunk-size 128 --tc 4`. These are starting points, not rules; more threads aren't always faster. Defaults and options are in the [flags table](#command-line-flags).

| Setup | Chunk (MiB) | Threads (`--tc`) | Buffer RAM, roughly |
|-------|-------------|-----------------|---------------------|
| SSD baseline | 128 | 4 | 128 MiB |
| Strong CPU + fast NVMe | 128 | 8 | 128 MiB |
| Input/output on the same HDD | 128 | 1 | 128 MiB |
| Low RAM | 16 | 1–2 | 16 MiB |

These numbers are for one ISO at a time! For example a 128 MiB chunk with two ISOs running needs about 256 MiB for the chunks, plus the app's other memory. The CPU threads are shared between the ISO. 

Bigger chunks use more RAM but mean fewer disk reads and writes. If you're reading and writing on the same HDD, stick to [--jobs 1](#command-line-flags) to avoid moving the drive head around as much.

### tests

The scripts in `tests/` generate one mock `Fake.iso` it has PS3 headers, and payload sectors encrypted by OpenSSL with a hardcoded test key they run the actual executable and compare the  decrypted image. 

FYI this test image isn't bootable.

Install `genisoimage` (or `mkisofs`), OpenSSL, and Python 3.11+, then run:

```sh
cargo build --locked --bin ps3dec
python3 -B tests/e2e.py
```

To generate just the fixture:

```sh
python3 -B tests/fake_iso.py
```

## Building PS3dec

This works for macOS, Linux and windows and only 64bit builds.

1. Install Rust from https://rustup.rs/
2. Clone the repository run ```git clone https://github.com/Redrrx/ps3dec
cd ps3dec```
3. make sure to close and reopen your terminal for proper rust install to be recognized
4. ```cargo build --release```

the output would be at /target
If on linux run ```chmod+x target/release/ps3dec``` to make ps3dec an executable.

<sub>a very small note around here, if there's an issue that is specifically related to a library  when it's targeting a platform I don't have much that I can do about it, but you can fork the repository and find a replacement or a custom implementation, but this is unlikely as most of the libraries used are not reliant on any critical platform specific implementations and mostly standard ie: win api etc...</sup>

### Building for a special platform?

Run cargo check to check for compatibility:  

`cargo check --target <target-triple>`

Use this command to add your new target platform

`rustup target add <target-triple>` 

Then build for the target using:  

`cargo build --release --target <target-triple>`

more on targets [here](https://doc.rust-lang.org/nightly/rustc/platform-support.html)


## Releases types

If you visit the releases page you might find two types

* Stable == ready to use, reliable enough.
* Preview == trying out requests, and toying around before stable.



## Acknowledgements

- [Aldostools PS3 ird Databases](https://ps3.aldostools.org/ird.html)
- [Psdevwiki Bluray information ](https://www.psdevwiki.com/ps3/Bluray_disc)
- [Understanding PS3 disk encryption](https://www.psx-place.com/threads/3k3y-iso-tools-understanding-ps3-disk-encryption.29903/)
- The people who open issues/suggestions when they have any :D
