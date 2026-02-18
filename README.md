# Barrel

Our shot to reimplement Lego Racers.

## Tools

* [jam.rs](src/bin/jam.rs) - extract the assets from a JAM file. 
* [bmp.rs](src/bin/bmp.rs) - transcode Lego Racers custom BMP files into correct BMP.

## JAM

The Lego Racers CD includes a file `data1.cab`. Extract that file using [unshield](https://github.com/twogood/unshield).

```$
$unshield x data1.cab -d /tmp/lego-racers
Cabinet: data1.cab
  extracting: /tmp/lego-racers/Program_Files_Group/knight.tun
  extracting: /tmp/lego-racers/Program_Files_Group/2nd.tun
  ...
  extracting: /tmp/lego-racers/Program_Files_Group/win.tun
  extracting: /tmp/lego-racers/Program_Files_Group/witch.tun
 --------  -------
          56 files
```

Now, extract the assets from `/tmp/lego-racers/Program_Files_Group/LEGO.JAM`:

```bash
$ cargo run \
  --bin jam \
  -- \
  --input /tmp/lego-racers/Program_Files_Group/LEGO.JAM \
  --output /tmp/lego-racers/assets
Extracted /tmp/lego-racers/Program_Files_Group/LEGO.JAM to /tmp/lego-racers/assets.
```
