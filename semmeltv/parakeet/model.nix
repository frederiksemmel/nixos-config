{ fetchurl, linkFarm }:

let
  file =
    name: hash:
    fetchurl {
      url = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/ed5d745623472f97cfc2e6c2916ee248798fc5a8/${name}";
      inherit hash;
    };
in
linkFarm "parakeet-tdt-0.6b-v3-int8" {
  "encoder.int8.onnx" =
    file "encoder.int8.onnx" "sha256-GvvmnB/9q+6UdV9C5Nrx2TVJExo9oRNWvahPzvDyxHk=";
  "decoder.int8.onnx" =
    file "decoder.int8.onnx" "sha256-AiKoYUcpnU3ukO35W20Tgn7bQq2LfJzwslgvAwk3wLU=";
  "joiner.int8.onnx" = file "joiner.int8.onnx" "sha256-y9AvNpbplElwER+0OdnD/TZagDdFZfE/K5xwB9JT/Jg=";
  "tokens.txt" = file "tokens.txt" "sha256-1YVEZ56kvGrFY9H1Ret9R0vWz6Rn8KbiwdwcfTfjw10=";
}
