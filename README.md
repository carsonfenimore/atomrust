# atomrust

Atomrust provides the foundational layer for AI-enabled raspberry pi cameras.

## Operation
Atomrust captures h264/rgb streams using and publishes them over RTSP.  It also provides the following features:
 - mqtt stats published in a Home Assistant-friendly format
 - object detection with tensorflowlite

When coupled with a high-quality camera module, such as a Sony Starvis based sensor, and an nvr such as BlueIris, this can provide an extremely robust, in-house security system.

## Requirements
  - 64-bit versions pi OS 
  - ~128MB of ram 
  - around 0.5 cores on a pi zero 2w. 
    TFLite processing can consume a user-selectable number of cores.  For a pi zero 2w, if atomrust is configured to use 2 threads processing is capped at around 4fps.
  - rpicam-apps build: v1.10.0 24906da670e9-dirty 04-11-2025 (10:35:52)
  - libcamera build: v0.5.2+99-bfd68f78

## Building

Before building remember to build/install libcamera/rpicam-apps. Once this is done you can build atomrust as follows:

```
  sudo apt install libssl-dev
  cargo build
```

[!WARNING] 
tflite seems to have a small build error, as reported here: https://github.com/conan-io/conan-center-index/issues/24538.  You can fix this by:

```
vim `find . -iname "spectrogram.cc"`
#include <cstdint>  // <-- add this line right above the line saying #include <assert.h>
```


## Running

Note: for objdet grab stock tflite model files
    - mobilenet v2: https://github.com/google-coral/edgetpu/raw/refs/heads/master/test_data/ssd_mobilenet_v2_coco_quant_postprocess.tflite
    - coco_labels.txt: https://raw.githubusercontent.com/google-coral/edgetpu/refs/heads/master/test_data/coco_labels.txt   

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
      intraperiod: 5
    mqtt:
      username: "mqttuser"
      password: "mqttpass"
      host: "<mqtt_broker_ip>"
      port: <mqtt_broker_port_usually_1883>
      obj_name: "atomcam"
    pipeline: 
      model_filename: "ssd_mobilenet_v2_coco_quant_postprocess.tflite"
      threshold: 0.6
      label_filename: "coco_labels.txt"
      num_threads: 2

Then run

    atomrust config.yaml

## Development Status
This project is under active development and isn't fully ready.   We hope to have an easily-deployable release soon.  


## Changelog
 - 0.1.2 
    - support for latest libcamlite-rs
    - make mqtt and pipeline stages optional - ommitting both results in a simple rtsp server doing no processing or reporting
 - 0.1.1 
	- added tflite (after accidentally deleting it before pushing the code)
	- tie together mqtt and tflite objdet - home assistant, ala mqtt discovery, should now know when an objdet occurs.
	  alarm clears 5 sec after nothing seen.
 - 0.1.0 
	- initial release performing parallel h264 rtsp streaming and no-op rgb (future feed for objdet)
