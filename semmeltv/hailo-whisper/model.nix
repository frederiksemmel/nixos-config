{ fetchurl, linkFarm }:

let
  huggingFace =
    file: hash:
    fetchurl {
      url = "https://huggingface.co/onnx-community/whisper-base/resolve/27a07b4b52b3a4c5886134bb4320e93e1a4bbc7c/${file}";
      inherit hash;
    };
in
linkFarm "whisper-base-hailo8l" {
  "encoder.hef" = fetchurl {
    url = "https://hailo-model-zoo.s3.eu-west-2.amazonaws.com/ModelZoo/Compiled/v2.20.0/hailo8l/whisper_base_5s_encoder.hef";
    hash = "sha256-V4xW3i8dBcuYYwu4rrwId6+oW0JzI10/ndc0Q6syhh8=";
  };
  "decoder_model_int8.onnx" =
    huggingFace "onnx/decoder_model_int8.onnx" "sha256-yFljb0Rj3Vqep1JUbGx7retIqz4is0DK/vCKFVUvyy4=";
  "decoder_with_past_model_int8.onnx" =
    huggingFace "onnx/decoder_with_past_model_int8.onnx" "sha256-3Q2f8cBmmSqo22/xlEtf1WYOley7yG4NoVgwDCPlIjI=";
  "tokenizer.json" =
    huggingFace "tokenizer.json" "sha256-J/xHa/5/FymUgL4ic/wGCOTVqZq6KrXexTdLRILRpWY=";
}
