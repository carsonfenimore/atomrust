# atomrust

Atomrust provides the foundational layer for AI-enabled security cameras.

## Operation
Atomrust combines RTSP (based on [https://github.com/oddity-ai/oddity-rtsp]) for streaming H.264 video to an NVR, such as BlueIris (https://blueirissoftware.com).  Simultaneously atomrust processes RGB streams locally on the camera in a pipeline similar to rpicam-apps.  Currently the pipeline has a single TFLite stage for object detection.  As objects are identified in the video stream they are immediately published over to HomeAssistant via MQTT [https://www.home-assistant.io].  This allow HA automations to react to object detection events.

## Requirements
  - Right now atomrust only works on 64-bit versions of raspberry pi OS.  It could theoretically run on 32-bit pi OS.
  - Processing: needs ~128MB of ram and consumes about 0.5 cores on a pi zero 2w. TFLite processing can consume a user-selectable number of cores.  For a pi zero 2w, if atomrust is configured to use 2 threads processing is capped at around 4fps.
  - libcamera v0.3.1+rpt20240906
  - rpicam-apps 1.5.1

## Installation
  - Build and install libcamera/rpicam-apps
  - Build and install atomrust
  - Grab stock tflite model files
    - mobilenet v2: https://github.com/google-coral/edgetpu/raw/refs/heads/master/test_data/ssd_mobilenet_v2_coco_quant_postprocess.tflite
    - coco_labels.txt: https://raw.githubusercontent.com/google-coral/edgetpu/refs/heads/master/test_data/coco_labels.txt   


## Running
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


## Development Status
This project is under active development and isn't fully ready.   We hope to have an easily-deployable release soon.  


Then run

    atomrust config.yaml
