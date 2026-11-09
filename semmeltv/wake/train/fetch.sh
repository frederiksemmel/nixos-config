#!/usr/bin/env bash
set -u
cd "$WAKE_WORK"
mkdir -p models voices rir
for m in melspectrogram embedding_model; do curl -sSfL -o models/$m.onnx https://github.com/dscripka/openWakeWord/releases/download/v0.5.1/$m.onnx; done
HF=https://huggingface.co/rhasspy/piper-voices/resolve/main
for v in en/en_US/libritts_r/medium/en_US-libritts_r-medium en/en_GB/vctk/medium/en_GB-vctk-medium en/en_US/l2arctic/medium/en_US-l2arctic-medium en/en_US/arctic/medium/en_US-arctic-medium en/en_GB/semaine/medium/en_GB-semaine-medium en/en_GB/aru/medium/en_GB-aru-medium en/en_US/hfc_female/medium/en_US-hfc_female-medium en/en_US/hfc_male/medium/en_US-hfc_male-medium en/en_US/ryan/high/en_US-ryan-high en/en_US/amy/medium/en_US-amy-medium en/en_US/joe/medium/en_US-joe-medium en/en_US/kristin/medium/en_US-kristin-medium en/en_US/norman/medium/en_US-norman-medium en/en_US/john/medium/en_US-john-medium en/en_US/bryce/medium/en_US-bryce-medium en/en_US/kusal/medium/en_US-kusal-medium en/en_US/ljspeech/high/en_US-ljspeech-high en/en_GB/northern_english_male/medium/en_GB-northern_english_male-medium en/en_GB/jenny_dioco/medium/en_GB-jenny_dioco-medium en/en_GB/alba/medium/en_GB-alba-medium en/en_GB/cori/high/en_GB-cori-high en/en_GB/southern_english_female/low/en_GB-southern_english_female-low de/de_DE/thorsten_emotional/medium/de_DE-thorsten_emotional-medium de/de_DE/eva_k/x_low/de_DE-eva_k-x_low de/de_DE/karlsson/low/de_DE-karlsson-low de/de_DE/pavoque/low/de_DE-pavoque-low de/de_DE/ramona/low/de_DE-ramona-low de/de_DE/mls/medium/de_DE-mls-medium es/es_ES/sharvard/medium/es_ES-sharvard-medium es/es_MX/ald/medium/es_MX-ald-medium es/es_AR/daniela/high/es_AR-daniela-high es/es_ES/carlfm/x_low/es_ES-carlfm-x_low; do
  n=$(basename $v)
  [ -s voices/$n.onnx ] || { curl -sSfL -o voices/$n.onnx "$HF/$v.onnx" && curl -sSfL -o voices/$n.onnx.json "$HF/$v.onnx.json" || echo "FAIL $n"; }
done
.venv/bin/python -c "
from huggingface_hub import snapshot_download
snapshot_download('davidscripka/MIT_environmental_impulse_responses', repo_type='dataset', local_dir='rir')
"
echo FETCH DONE
