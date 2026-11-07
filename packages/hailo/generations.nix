let
  github = owner: repo: rev: hash: {
    inherit
      owner
      repo
      rev
      hash
      ;
  };

  commonExternals = {
    protobuf =
      github "protocolbuffers" "protobuf" "f0dc78d7e6e331b8c6bb2d5283e06aa26883ca7c"
        "sha256-VZQEFHq17UsTH5CZZOcJBKiScGV2xPJ/e6gkkVliRCU=";
    spdlog =
      github "gabime" "spdlog" "27cb4c76708608465c413f6d0e6b8d99a4d84302"
        "sha256-F7khXbMilbh5b+eKnzcB0fPPWQqUHqAYPWJb83OnUKQ=";
    readerwriterqueue =
      github "cameron314" "readerwriterqueue" "435e36540e306cac40fcfeab8cc0a22d48464509"
        "sha256-dMHT21htCuMiu2sonYEPigmPFpk7a6OYMYXPCuTLA2o=";
    xxhash =
      github "Cyan4973" "xxHash" "bbb27a5efb85b92a0486cf361a8635715a53f6ba"
        "sha256-kofPs01jb189LUjYHHt+KxDifZQWl0Hm779711mvWtI=";
    dotwriter =
      github "hailo-ai" "DotWriter" "e5fa8f281adca10dd342b1d32e981499b8681daf"
        "sha256-1cJ23AqM+9aYxGVM07nkkY77xClii4znVMCFbf+Sdio=";
    pybind11 =
      github "pybind" "pybind11" "a2e59f0e7065404b44dfe92a28aca47ba1378dc4"
        "sha256-SNLdtrOjaC3lGHN9MAqTf51U9EzNKQLyTMNPe0GcdrU=";
    eigen = {
      gitlab = true;
      owner = "libeigen";
      repo = "eigen";
      rev = "3147391d946bb4b6c68edd901f2add6ac1f31f8c";
      hash = "sha256-1/4xMetKMDOgZgzz3WMxfHUEpmdAm52RqZvz6i0mLEw=";
    };
  };
in
{
  hailo8 = {
    version = "4.24.0";
    hailortHash = "sha256-NMjAtIyQkX/cJoI7JjB8KRWMJGG4Pzqw0hDXkyR6kQg=";
    driverHash = "sha256-2yEKd6n/qReRXGIGMp3LjE0hM32qvaqOiIFxhT5h+yQ=";
    firmware = {
      path = "Hailo8/4.24.0/FW/hailo8_fw.4.24.0.bin";
      hash = "sha256-Pr6yCZh7sOSENTviN3TTiTkI3oQGkBbE43NhyHHYQzo=";
      licenseUrl = "https://github.com/hailo-ai/hailort/blob/v4.24.0/hailort/hailo_firmware_eula";
    };
    pythonDependencies = [
      "argcomplete"
      "netaddr"
      "netifaces"
      "numpy"
    ];
    kernelModule = "hailo_pci";
    driverDir = "linux/pcie";
    deviceClass = "hailo_chardev";
    driverMakeFlags = [ ];
    externals = commonExternals // {
      cli11 =
        github "hailo-ai" "CLI11" "ae78ac41cf225706e83f57da45117e3e90d4a5b4"
          "sha256-KGoPg8vIIq3Ue0VLrmz/rxWVkDJ+27XWJki+iJ0o7uw=";
      json =
        github "ArthurSonzogni" "nlohmann_json_cmake_fetchcontent"
          "391786c6c3abdd3eeb993a3154f1f2a4cfe137a0"
          "sha256-5A18zFqbgDc99pqQUPcpwHi89WXb8YVR9VEwO18jH2I=";
    };
  };

  hailo10h = {
    version = "5.5.0";
    hailortHash = "sha256-D5UP0z3kGr35/wwSVEyGKNQg57zei4ABo5xFbPV2KOo=";
    driverHash = "sha256-6G7mrxR/8SC5AV00NDvwc3fLBJiXn+68bUv+rFcJjdo=";
    firmware = {
      path = "Hailo10H/5.5.0/FW/hailo10h_fw.tar.gz";
      hash = "sha256-QZRSLHXEp9ziwa6+YcXYr/LzVLPd4HxGYRMIc5RTP8o=";
      licenseUrl = "https://github.com/hailo-ai/hailort-drivers/blob/v5.5.0/Hailo10H%20FW%20License";
    };
    pythonDependencies = [ "numpy" ];
    kernelModule = "hailo1x_pci";
    driverDir = ".";
    deviceClass = "hailo1x";
    driverMakeFlags = [ "HAILO_DRIVER=pcie" ];
    externals = commonExternals // {
      cli11 =
        github "hailo-ai" "CLI11" "242adfdb23957d30e3e56831e474020d0ac6c86c"
          "sha256-qFzUIpZGIbYWtFIew9RLXgYdApRXIonq4qOiTn6Xrbc=";
      json =
        github "nlohmann" "json" "9cca280a4d0ccf0c08f47a99aa71d1b0e52f8d03"
          "sha256-7F0Jon+1oWL7uqet5i1IgHX0fUw/+z0QwEcA3zs5xHg=";
      tl-expected =
        github "TartanLlama" "expected" "1770e3559f2f6ea4a5fb4f577ad22aeb30fbd8e4"
          "sha256-+9M1++ZYaZNJGjEoI6+J8565R2wlznoDWW8MPrmCMoU=";
      minja =
        github "google" "minja" "58568621432715b0ed38efd16238b0e7ff36c3ba"
          "sha256-kvFZdjpbRji2zaDo30+cBCPNa4arhFAQhSTfkt9SgRo=";
    };
  };
}
