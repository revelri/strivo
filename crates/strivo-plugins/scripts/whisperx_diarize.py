#!/usr/bin/env python3
"""Two-stage WhisperX orchestrator for StriVo CrunchR.

WhisperX's default pipeline keeps the Whisper ASR model, alignment model, AND
the pyannote diarization pipeline resident on the GPU simultaneously — that
peaks well over 8 GB VRAM for `large-v3`. StriVo's `whisperx-local` backend
prefers an 8 GB GPU, so this orchestrator unloads each stage before loading
the next, explicitly forcing `gc.collect()` + `torch.cuda.empty_cache()` to
drop the freed allocations.

Stages
------

1.  **Whisper transcription** (≈5–6 GB VRAM for large-v3 fp16).
2.  **Alignment** (wav2vec, ≈1 GB) — gives us word-level timestamps that
    pyannote's diarization can join against.
3.  **pyannote 3.1 diarization** (≈2.5 GB) — produces speaker turns.
4.  Speaker turns are joined back onto the aligned segments via
    `whisperx.assign_word_speakers`.

The result is written as JSON to the path passed on the command line. Rust
side parses it back into `Segment{ speaker, start, end, text, confidence }`.

Usage
-----

::

    whisperx_diarize.py <input.wav> <output.json> \
        [--model large-v3] [--compute-type float16] [--language en] \
        [--diarize/--no-diarize]

Environment
-----------

* ``HF_TOKEN`` (or whatever ``CrunchrConfig.api_key_env`` resolves to) —
  required only when ``--diarize`` is set, since pyannote needs the
  HuggingFace license token to fetch its model weights.
* ``CUDA_VISIBLE_DEVICES`` — honoured transparently by torch.
"""

from __future__ import annotations

import argparse
import gc
import json
import os
import sys
import traceback
from pathlib import Path


def _free_cuda() -> None:
    """Best-effort drop of any CUDA allocations held by torch."""
    try:
        import torch  # noqa: WPS433 — local to keep startup fast

        gc.collect()
        if torch.cuda.is_available():
            torch.cuda.empty_cache()
            torch.cuda.synchronize()
    except Exception:  # noqa: BLE001
        # torch may not be importable on CPU-only debug runs; nothing to do.
        pass


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="StriVo CrunchR WhisperX orchestrator")
    parser.add_argument("audio", help="Input audio (wav/flac/mp3)")
    parser.add_argument("output", help="Where to write the result JSON")
    parser.add_argument(
        "--model",
        default=os.environ.get("STRIVO_WHISPERX_MODEL", "large-v3"),
        help="Whisper model size (default: large-v3)",
    )
    parser.add_argument(
        "--compute-type",
        default=os.environ.get("STRIVO_WHISPERX_COMPUTE", "float16"),
        choices=["float16", "int8", "float32"],
        help="Compute dtype (default: float16 — fp16 fits 8 GB GPUs)",
    )
    parser.add_argument(
        "--language",
        default=None,
        help="Force language code (e.g. 'en'). Default: auto-detect.",
    )
    parser.add_argument(
        "--batch-size",
        type=int,
        default=int(os.environ.get("STRIVO_WHISPERX_BATCH", "8")),
        help="Whisper transcribe batch size (default: 8 — safe on 8 GB).",
    )
    parser.add_argument(
        "--diarize",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Run pyannote diarization after transcription (default: yes).",
    )
    parser.add_argument(
        "--device",
        default=os.environ.get("STRIVO_WHISPERX_DEVICE", "cuda"),
        help="cuda | cpu (default: cuda).",
    )
    args = parser.parse_args(argv)

    try:
        import whisperx  # noqa: WPS433
    except ImportError as exc:
        print(
            f"whisperx_diarize: whisperx is not installed: {exc}\n"
            "Install with: pip install whisperx",
            file=sys.stderr,
        )
        return 2

    audio_path = Path(args.audio)
    if not audio_path.exists():
        print(f"whisperx_diarize: audio not found: {audio_path}", file=sys.stderr)
        return 2

    # ── Stage 1 ── Whisper transcription ─────────────────────────────────────
    print(
        f"whisperx_diarize: stage 1 — loading {args.model} on {args.device} "
        f"({args.compute_type})",
        file=sys.stderr,
    )
    asr_model = whisperx.load_model(
        args.model,
        args.device,
        compute_type=args.compute_type,
    )
    audio = whisperx.load_audio(str(audio_path))
    result = asr_model.transcribe(audio, batch_size=args.batch_size, language=args.language)
    detected_language = result.get("language", args.language or "en")
    # IMPORTANT: drop the ASR model before loading the alignment model so the
    # next stage has VRAM headroom. Forcing gc.collect + empty_cache here is
    # what makes 8 GB GPUs viable.
    del asr_model
    _free_cuda()

    # ── Stage 2 ── word-level alignment ──────────────────────────────────────
    print(
        f"whisperx_diarize: stage 2 — alignment (lang={detected_language})",
        file=sys.stderr,
    )
    align_model, align_meta = whisperx.load_align_model(
        language_code=detected_language,
        device=args.device,
    )
    aligned = whisperx.align(
        result["segments"],
        align_model,
        align_meta,
        audio,
        args.device,
        return_char_alignments=False,
    )
    del align_model
    _free_cuda()

    segments_out: list[dict] = aligned.get("segments", [])

    # ── Stage 3 ── diarization (optional) ────────────────────────────────────
    if args.diarize:
        hf_token = os.environ.get("HF_TOKEN") or os.environ.get("HUGGINGFACE_TOKEN")
        if not hf_token:
            print(
                "whisperx_diarize: --diarize requested but no HF_TOKEN env var; "
                "skipping diarization. Set HF_TOKEN to a HuggingFace token that "
                "has accepted the pyannote/speaker-diarization-3.1 license.",
                file=sys.stderr,
            )
        else:
            print(
                "whisperx_diarize: stage 3 — pyannote diarization",
                file=sys.stderr,
            )
            diarize_pipeline = whisperx.DiarizationPipeline(
                use_auth_token=hf_token,
                device=args.device,
            )
            diarize_segments = diarize_pipeline(audio)
            assigned = whisperx.assign_word_speakers(diarize_segments, aligned)
            segments_out = assigned.get("segments", segments_out)
            del diarize_pipeline
            _free_cuda()

    # Normalise output. Aligned segments expose `start`, `end`, `text`, and
    # (when diarized) `speaker`. We emit the shape the Rust side already
    # understands (matches our voxtral-local / voxtral-openrouter schema).
    normalised = []
    full_text_parts: list[str] = []
    for idx, seg in enumerate(segments_out):
        text = (seg.get("text") or "").strip()
        speaker = seg.get("speaker")
        normalised.append(
            {
                "index": idx,
                "start": float(seg.get("start") or 0.0),
                "end": float(seg.get("end") or 0.0),
                "text": text,
                "speaker": speaker,
                "confidence": seg.get("avg_logprob"),
            }
        )
        if text:
            full_text_parts.append(text)

    payload = {
        "language": detected_language,
        "text": " ".join(full_text_parts),
        "segments": normalised,
    }
    Path(args.output).write_text(json.dumps(payload), encoding="utf-8")
    print(
        f"whisperx_diarize: wrote {len(normalised)} segments to {args.output}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":  # pragma: no cover
    try:
        raise SystemExit(main(sys.argv[1:]))
    except Exception:  # noqa: BLE001
        traceback.print_exc()
        sys.exit(1)
