# PS3 Decryptor

PS3Dec is a remake of the original PS3 decryptor which decrypts PS3s Redump ISOs.

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

## Usage

--iso is for your iso file --dk is for your decryption key and --tc is for thread count.

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

## Demo

[![Alt text for the thumbnail](URL_of_Thumbnail)](URL_of_Video "Video Title")

## Acknowledgements

- [Aldostools PS3 ird Databases](https://ps3.aldostools.org/ird.html)
- [Psdevwiki Bluray information ](https://www.psdevwiki.com/ps3/Bluray_disc)
- [Understanding PS3 disk encryption](https://www.psx-place.com/threads/3k3y-iso-tools-understanding-ps3-disk-encryption.29903/)

## License


