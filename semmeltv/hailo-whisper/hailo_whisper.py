"""Wyoming speech-to-text server running Whisper's encoder on a Hailo-8L and its decoder on the CPU.

The spoken language is detected per command, among the ones the TV understands."""

import argparse
import asyncio
import logging
import threading
import time
from functools import partial
from pathlib import Path

import numpy as np
import onnxruntime as ort
from faster_whisper.feature_extractor import FeatureExtractor
from hailo_platform import FormatType, HailoSchedulingAlgorithm, VDevice
from tokenizers import Tokenizer
from wyoming.asr import Transcribe, Transcript
from wyoming.audio import AudioChunk, AudioChunkConverter, AudioStart, AudioStop
from wyoming.event import Event
from wyoming.info import AsrModel, AsrProgram, Attribution, Describe, Info
from wyoming.server import AsyncEventHandler, AsyncServer

RATE = 16000
HOP = 160
MAX_TOKENS = 48
LANGUAGES = ("en", "de", "es")
LOG = logging.getLogger("hailo-whisper")


class Whisper:
    def __init__(self, model: Path, threads: int):
        params = VDevice.create_params()
        params.scheduling_algorithm = HailoSchedulingAlgorithm.ROUND_ROBIN
        self.device = VDevice(params)
        self.encoder = self.device.create_infer_model(str(model / "encoder.hef"))
        self.encoder.input().set_format_type(FormatType.FLOAT32)
        self.encoder.output().set_format_type(FormatType.FLOAT32)
        self.configured = self.encoder.configure()
        self.bindings = self.configured.create_bindings()
        self.input_shape = self.encoder.input().shape
        self.output_shape = self.encoder.output().shape
        frames = self.input_shape[-2]
        self.samples = frames * HOP
        self.features = FeatureExtractor(feature_size=self.input_shape[-1], chunk_length=self.samples // RATE)

        options = ort.SessionOptions()
        options.intra_op_num_threads = threads
        options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
        session = partial(ort.InferenceSession, sess_options=options, providers=["CPUExecutionProvider"])
        self.first = session(str(model / "decoder_model_int8.onnx"))
        self.step = session(str(model / "decoder_with_past_model_int8.onnx"))
        self.first_outputs = [output.name for output in self.first.get_outputs()]
        self.step_outputs = [output.name for output in self.step.get_outputs()]

        self.tokenizer = Tokenizer.from_file(str(model / "tokenizer.json"))
        token = self.tokenizer.token_to_id
        self.start = token("<|startoftranscript|>")
        self.languages = {name: token(f"<|{name}|>") for name in LANGUAGES}
        self.task = [token("<|transcribe|>"), token("<|notimestamps|>")]
        self.end = token("<|endoftext|>")
        self.lock = threading.Lock()

    def mel(self, audio: np.ndarray) -> np.ndarray:
        audio = audio[: self.samples]
        audio = np.pad(audio, (0, self.samples - len(audio)))
        features = self.features(audio, padding=0)[:, : self.input_shape[-2]]
        return np.ascontiguousarray(features.T.reshape(self.input_shape), dtype=np.float32)

    def encode(self, mel: np.ndarray) -> np.ndarray:
        self.bindings.input().set_buffer(mel)
        hidden = np.empty(self.output_shape, dtype=np.float32)
        self.bindings.output().set_buffer(hidden)
        self.configured.run([self.bindings], 10_000)
        return hidden.reshape(1, *self.output_shape[-2:])

    def first_pass(self, prompt: list[int], hidden: np.ndarray) -> list[np.ndarray]:
        return self.first.run(None, {"input_ids": np.array([prompt], np.int64), "encoder_hidden_states": hidden})

    def language(self, hidden: np.ndarray) -> str:
        logits = self.first_pass([self.start], hidden)[0][0, -1]
        return max(LANGUAGES, key=lambda name: logits[self.languages[name]])

    def decode(self, hidden: np.ndarray, language: str) -> str:
        outputs = self.first_pass([self.start, self.languages[language], *self.task], hidden)
        past = cache(self.first_outputs, outputs)
        tokens = []
        for _ in range(MAX_TOKENS):
            token = int(np.argmax(outputs[0][0, -1]))
            if token >= self.end:
                break
            tokens.append(token)
            if repeated(tokens):
                break
            outputs = self.step.run(None, {"input_ids": np.array([[token]], np.int64), **past})
            past.update(cache(self.step_outputs, outputs))
        return self.tokenizer.decode(tokens, skip_special_tokens=True).strip()

    def transcribe(self, audio: np.ndarray) -> str:
        with self.lock:
            started = time.monotonic()
            mel = self.mel(audio)
            featured = time.monotonic()
            hidden = self.encode(mel)
            encoded = time.monotonic()
            language = self.language(hidden)
            detected = time.monotonic()
            text = self.decode(hidden, language)
            done = time.monotonic()
        LOG.info(
            "%.2fs of audio: features %.0f ms, encoder %.0f ms, language %s in %.0f ms, decoder %.0f ms: %r",
            len(audio) / RATE,
            (featured - started) * 1000,
            (encoded - featured) * 1000,
            language,
            (detected - encoded) * 1000,
            (done - detected) * 1000,
            text,
        )
        return text


def repeated(tokens: list[int]) -> bool:
    for size in range(2, len(tokens) // 2 + 1):
        if tokens[-size:] == tokens[-2 * size : -size]:
            del tokens[-size:]
            return True
    return False


def cache(names: list[str], outputs: list[np.ndarray]) -> dict[str, np.ndarray]:
    return {
        name.replace("present.", "past_key_values."): value
        for name, value in zip(names[1:], outputs[1:])
        if name.startswith("present.")
    }


class Handler(AsyncEventHandler):
    def __init__(self, whisper: Whisper, info: Info, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.whisper = whisper
        self.info = info
        self.converter = AudioChunkConverter(rate=RATE, width=2, channels=1)
        self.audio = bytearray()

    async def handle_event(self, event: Event) -> bool:
        if Describe.is_type(event.type):
            await self.write_event(self.info.event())
        elif Transcribe.is_type(event.type) or AudioStart.is_type(event.type):
            self.audio.clear()
        elif AudioChunk.is_type(event.type):
            self.audio.extend(self.converter.convert(AudioChunk.from_event(event)).audio)
        elif AudioStop.is_type(event.type):
            samples = np.frombuffer(bytes(self.audio), dtype=np.int16).astype(np.float32) / 32768.0
            self.audio.clear()
            loop = asyncio.get_running_loop()
            text = await loop.run_in_executor(None, self.whisper.transcribe, samples)
            await self.write_event(Transcript(text=text).event())
        return True


async def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uri", required=True, help="tcp://HOST:PORT or unix://PATH")
    parser.add_argument(
        "--model", required=True, type=Path, help="directory with encoder.hef, the ONNX decoders and tokenizer.json"
    )
    parser.add_argument("--threads", type=int, default=4)
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")

    whisper = Whisper(args.model, args.threads)
    whisper.transcribe(np.zeros(RATE, dtype=np.float32))
    attribution = Attribution(name="OpenAI", url="https://github.com/openai/whisper")
    model = AsrModel(
        name="whisper-hailo",
        description="Whisper, encoder on Hailo-8L",
        attribution=attribution,
        installed=True,
        version=None,
        languages=list(LANGUAGES),
    )
    program = AsrProgram(
        name="hailo-whisper",
        description=__doc__,
        attribution=attribution,
        installed=True,
        version=None,
        models=[model],
    )
    info = Info(asr=[program])
    LOG.info("ready on %s", args.uri)
    server = AsyncServer.from_uri(args.uri)
    await server.run(partial(Handler, whisper, info))


def run() -> None:
    asyncio.run(main())


if __name__ == "__main__":
    run()
