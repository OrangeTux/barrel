# Barrel

Our shot to reimplement Lego Racers.

## Tools

* [extract.rs](src/bin/extract.rs) - extract al files from a InstallShield archive.
* [jam.rs](src/bin/jam.rs) - extract the assets from a JAM file. 
* [bmp.rs](src/bin/bmp.rs) - transcode Lego Racers custom BMP files into correct BMP.

## JAM

The Lego Racers CD includes the file `data1.hdr`. 

```bash
$ cargo run \
  --bin extract \
  -- \
  --input /tmp/lego-racers/data1.hdr \
  --output /tmp/lego-racers

Extracted "/tmp/Program Files Group/knight.tun".
Extracted "/tmp/Program Files Group/2nd.tun".
...
Extracted "/tmp/Program Files Group/win.tun".
Extracted "/tmp/Program Files Group/witch.tun".
Extracted 56 files.
```

Now, extract the assets from `/tmp/lego-racers/Program_Files_Group/LEGO.JAM`:

```bash
$ cargo run \
  --bin jam \
  -- \
  --input "/tmp/lego-racers/Program Files Group/LEGO.JAM" \
  --output /tmp/lego-racers/assets
Extracted /tmp/lego-racers/Program_Files_Group/LEGO.JAM to /tmp/lego-racers/assets.
```
