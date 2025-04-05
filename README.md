# PS3 Decryptor

PS3Dec is a remake of the original PS3 decryptor which decrypts PS3s redump ISOs.

the original one was written in C around 11 years ago, the sole reason i rewrote this one is for learning Rust and making my own slightly faster version to add features later anytime i want.

also i love ps3.


## How does it work ?

According to [PSDev Wiki](https://www.psdevwiki.com/ps3/Bluray_disc)
a Bluray disc consists of sectors with a lenght of
2048 bytes.

Encryption:

- Some regions are encrypted some are not
- Usually even numbered regions are encrypted and odd numbered regions are not encrypted
- the encryption used is AES-128 in CBC mode with no padding

What is simply done is using a decryption key and decrypting what needs to be decrypted as for the rest its directly
written to disk without
keeping the data in memory.


## Demo

Decrypting MX vs. ATV Untamed (USA) in less than 2 seconds on a fast enough rig! sometimes increasing the thread count too high might add a slight overhead for the dec process to start.



Please bear in mind this demonstration is done on some very idealistic conditions with a very good CPU and a good SSD.



https://github.com/user-attachments/assets/978c1827-d788-449a-a52f-6743e94cb4db



## Usage

| Option   | Description                                         | Note                                                      |
|----------|-----------------------------------------------------|-----------------------------------------------------------|
| `--iso`  | For the ISO file                                    |                                                           |
| `--dk`   | For decryption key, a base-16 hex key               |                                                           |
| `--tc`   | Thread count, specifies the number of threads       | Be careful with this one                                  |
| `--auto` | Enables automatic key detection and decryption      | Will only work if there is the key in the **keys** folder |
| `--skip` | Disables the press any key to exit after decryption |                                                           |    



```
ps3dec.exe --iso game.iso --dk yourdecryptionkey --tc 64
```

If you don't want to keep changing your decryption key every time you can use --auto flag , which will look
inside a folder called **keys** containing the entire PS3 keys library which you can fetch from aldostools dkey database
here  [Aldostools dkeys](https://ps3.aldostools.org/dkey.html)  , to note that only .dkey files containing a base-16 hex
key are compatible.

```
ps3dec.exe --iso game.iso --auto --tc 64
```

## Building PS3dec

This works for MacOS, Linux and windows

1. Install Rust from https://rustup.rs/
2. Clone the repository run ```git clone https://github.com/Redrrx/ps3dec
cd ps3dec```
3. make sure to close and reopen your terminal for proper rust install to be recognized
4. ```cargo build --release```

the output would be at /target
If on linux run ```chmod+x target/release/ps3dec``` to make ps3dec an executable.

<sub>a very small note around here, if there's an issue that is specifically related to a library  when its targeting a platform i don't have much i can do about it but you can fork the repository and find a replacement or a custom implementation, but this is unlikely as most of the libraries used are not reliant on any critical platform specific implementations and mostly standard ie: win api etc...</sup>

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



