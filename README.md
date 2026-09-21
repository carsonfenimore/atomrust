# atomrust

Atomrust provides the foundational layer for AI-enabled raspberry pi cameras.  It efficiently (without decoding) streams RGB frames to tflite while streaming H.264 via RTSP.  Sensor stats can also be pushed via MQTT.  It does this efficiently, requiring only 10% CPU on pi zero 2w (excluding tflite processing)

When coupled with a high-quality camera module, such as a Sony Starvis based sensor, and an nvr such as BlueIris, this can provide an extremely robust, in-house security system.

## Requirements
  - 64-bit versions pi OS 
  - ~128MB of ram 
  - around 0.5 cores on a pi zero 2w. 
    TFLite processing can consume a user-selectable number of cores.  For a pi zero 2w, if atomrust is configured to use 2 threads processing is capped at around 4fps.
  - rpicam-apps build: v1.10.0 24906da670e9-dirty 04-11-2025 (10:35:52)
  - libcamera build: v0.5.2+99-bfd68f78

## Building a .deb (cross-compile)

On an x86_64 Linux host with podman (or docker):

```
packaging/build-deb.sh               # -> dist/atomrust_<version>_arm64.deb
packaging/build-deb.sh --no-objdet   # without TFLite object detection (faster first build)
```

This builds a Debian trixie container with the arm64 libcamera / rpicam-apps / ffmpeg
dev packages from the Raspberry Pi archive (`packaging/Containerfile`) and cross-compiles
a release binary. libcamlite is compiled in statically (by the rslibcamlite crate), so
no `LD_LIBRARY_PATH` is needed. The same package runs on any 64-bit Pi OS trixie board
(Zero 2 W, 3, 4, 5). Keep the Pi's packages current (`apt full-upgrade`): the package
depends on the libcamera/rpicam-apps versions it was built against.

## Installing

```
scp dist/atomrust_*_arm64.deb pi:
ssh pi sudo apt install ./atomrust_*_arm64.deb
```

Don't stage the .deb in `/tmp`; it is a RAM disk on trixie and is lost on reboot.

The package installs:
  - `atomrust.service` (enabled, runs as the unprivileged `atomrust` user, restarts on failure)
  - `/etc/atomrust/config.yml` (edit, then `sudo systemctl restart atomrust`)
  - models in `/usr/share/atomrust/models`
  - `atomrust-ro` / `atomrust-rw` / `atomrust-overlay status`

Logs: `journalctl -u atomrust -f` (set `Environment=LOG=debug` via `systemctl edit atomrust` for more).

## Read-only SD card mode

```
sudo atomrust-ro --reboot     # SD card mounted read-only; all writes go to RAM
atomrust-overlay status
```

Read-only mode uses the Debian `overlayroot` initramfs hook (like raspi-config's overlay
option). It also mounts `/boot/firmware` read-only, keeps the journal in RAM (32 MB cap),
switches rpi-swap to zram-only (no writeback file on the card) and disables the apt,
man-db, dpkg-db-backup and e2scrub timers. Changes made while read-only, including
config edits and package installs, are lost at reboot.

To update:

```
sudo atomrust-rw --reboot
sudo apt install ./atomrust_<new>_arm64.deb     # and/or edit /etc/atomrust/config.yml
sudo atomrust-ro --reboot
```

## Running from source

Note: for objdet we have included mobilenetv2/coco labels inside the models subdir

Populate a config.yaml, such as the following

    server:
      host: 0.0.0.0
      port: 5554
    camera:
      rtsppath: "/video"
      width: 1920
      height: 1080
      lowres_width: 300
      lowres_height: 300
      framerate: 30
      bitrate: "4mbps"
      profile: "main"
      intraperiod: 30
    mqtt:
      username: "mqttuser"
      password: "mqttpass"
      host: "<mqtt_broker_ip>"
      port: <mqtt_broker_port_usually_1883>
      obj_name: "atomcam"
    pipeline: 
      model_filename: "models/ssd_mobilenet_v2_coco_quant_postprocess.tflite"
      threshold: 0.6
      label_filename: "models/coco_labels.txt"
      num_threads: 2

Then run

    ./target/release/atomrust config.yaml

Any setting can be overridden from the environment, e.g. `ATOMRUST_CAMERA__BITRATE=6mbps`.

## Development Status
This project is under active development and isn't fully ready.   We hope to have an easily-deployable release soon.  


## Changelog
 - 0.2.0
    - .deb packaging via cross-compilation, systemd service, read-only SD mode (atomrust-ro / atomrust-rw)
    - release build (opt + LTO, stripped); libcamlite linked statically against distro libcamera/rpicam-apps
    - stability: bounded per-client queues (a stalled client can no longer exhaust memory),
      sessions survive lag and resync on keyframes, write timeouts, frame-stall watchdog,
      startup timeout when no camera, SIGTERM handling, no panics in camera callbacks
    - RTP timestamps from the sensor clock; streams start on a keyframe
    - MQTT: reconnects, never blocks the runtime, discovery sent once (retained), expire_after
    - fixes: mem_free was stale and reported used %, network rates were per-5s not per-second
    - TFLite object detection is an optional (default) `objdet` cargo feature
 - 0.1.2 
    - support for latest libcamlite-rs
    - make mqtt and pipeline stages optional - ommitting both results in a simple rtsp server doing no processing or reporting
 - 0.1.1 
	- added tflite (after accidentally deleting it before pushing the code)
	- tie together mqtt and tflite objdet - home assistant, ala mqtt discovery, should now know when an objdet occurs.
	  alarm clears 5 sec after nothing seen.
 - 0.1.0 
	- initial release performing parallel h264 rtsp streaming and no-op rgb (future feed for objdet)
