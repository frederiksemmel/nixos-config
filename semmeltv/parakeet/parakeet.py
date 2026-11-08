"""Wyoming speech-to-text server running NVIDIA Parakeet TDT 0.6B v3 on the CPU through sherpa-onnx.

Parakeet finds the spoken language itself, among 25 European languages."""

import argparse
import asyncio
import logging
import threading
import time
from functools import partial
from pathlib import Path

import numpy as np
import sherpa_onnx
from wyoming.asr import Transcribe, Transcript
from wyoming.audio import AudioChunk, AudioChunkConverter, AudioStart, AudioStop
from wyoming.event import Event
from wyoming.info import AsrModel, AsrProgram, Attribution, Describe, Info
from wyoming.server import AsyncEventHandler, AsyncServer

RATE = 16000
LANGUAGES = [
    "bg", "cs", "da", "de", "el", "en", "es", "et", "fi", "fr", "hr", "hu", "it",
    "lt", "lv", "mt", "nl", "pl", "pt", "ro", "ru", "sk", "sl", "sv", "uk",
]
LOG = logging.getLogger("parakeet")


class Parakeet:
    def __init__(self, model: Path, threads: int):
        self.recognizer = sherpa_onnx.OfflineRecognizer.from_transducer(
            encoder=str(model / "encoder.int8.onnx"),
            decoder=str(model / "decoder.int8.onnx"),
            joiner=str(model / "joiner.int8.onnx"),
            tokens=str(model / "tokens.txt"),
            num_threads=threads,
            model_type="nemo_transducer",
            decoding_method="greedy_search",
        )
        self.lock = threading.Lock()

    def transcribe(self, audio: np.ndarray) -> str:
        with self.lock:
            started = time.monotonic()
            stream = self.recognizer.create_stream()
            stream.accept_waveform(RATE, audio)
            self.recognizer.decode_stream(stream)
            text = stream.result.text.strip()
        LOG.info("%.2fs of audio in %.0f ms: %r", len(audio) / RATE, (time.monotonic() - started) * 1000, text)
        return text


class Handler(AsyncEventHandler):
    def __init__(self, parakeet: Parakeet, info: Info, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.parakeet = parakeet
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
            text = await loop.run_in_executor(None, self.parakeet.transcribe, samples)
            await self.write_event(Transcript(text=text).event())
        return True


async def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uri", required=True, help="tcp://HOST:PORT or unix://PATH")
    parser.add_argument("--model", required=True, type=Path, help="directory with the int8 transducer and tokens.txt")
    parser.add_argument("--threads", type=int, default=4)
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")

    parakeet = Parakeet(args.model, args.threads)
    parakeet.transcribe(np.zeros(RATE, dtype=np.float32))
    attribution = Attribution(name="NVIDIA", url="https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3")
    model = AsrModel(
        name="parakeet-tdt-0.6b-v3",
        description="Parakeet TDT 0.6B v3, int8",
        attribution=attribution,
        installed=True,
        version=None,
        languages=LANGUAGES,
    )
    program = AsrProgram(
        name="parakeet-wyoming",
        description=__doc__,
        attribution=attribution,
        installed=True,
        version=None,
        models=[model],
    )
    LOG.info("ready on %s", args.uri)
    server = AsyncServer.from_uri(args.uri)
    await server.run(partial(Handler, parakeet, Info(asr=[program])))


def run() -> None:
    asyncio.run(main())


if __name__ == "__main__":
    run()
