#!/usr/bin/env python3
"""随机生成日志，按批次持续追加到文件；按 Ctrl+C 停止。"""

import argparse
import math
import random
import sys
import time
from datetime import datetime, timezone
from pathlib import Path


LEVELS = ("TRACE", "DEBUG", "INFO", "WARN", "ERROR")
SERVICES = ("gateway", "auth", "billing", "catalog", "worker", "scheduler")
EVENTS = (
    "request completed",
    "cache entry refreshed",
    "message acknowledged",
    "database query finished",
    "retry scheduled",
    "upstream response received",
    "background job completed",
    "connection state changed",
)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="持续追加随机日志，每批刷新到文件，按 Ctrl+C 停止。"
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "target/test-data/live.log",
        help="输出文件（默认：仓库下的 target/test-data/live.log；存在则追加）",
    )
    parser.add_argument("--min-lines", type=int, default=1, help="每批最少行数（默认：1）")
    parser.add_argument("--max-lines", type=int, default=20, help="每批最多行数（默认：20）")
    parser.add_argument(
        "--interval", type=float, default=1.0, help="每批写入后等待的秒数（默认：1）"
    )
    parser.add_argument(
        "--total-lines", type=int, default=0, help="本次最多追加的行数，0 表示持续写入（默认：0）"
    )
    parser.add_argument("--seed", type=int, help="固定随机种子（时间戳仍使用当前时间）")
    return parser


def build_log_line(rng: random.Random, line_number: int) -> str:
    timestamp = datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")
    return (
        f"{timestamp} {rng.choice(LEVELS):<5} service={rng.choice(SERVICES)} "
        f"line={line_number} request_id={rng.getrandbits(64):016x} "
        f"status={rng.choice((200, 200, 201, 204, 400, 404, 429, 500, 503))} "
        f"latency_ms={rng.randint(1, 5000)} event={rng.choice(EVENTS)}\n"
    )


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    if args.min_lines < 1 or args.max_lines < args.min_lines:
        parser.error("需要满足 1 <= --min-lines <= --max-lines")
    if not math.isfinite(args.interval) or args.interval <= 0:
        parser.error("--interval 必须是大于 0 的有限数字")
    if args.total_lines < 0:
        parser.error("--total-lines 不能小于 0")

    rng = random.Random(args.seed)
    output_path = args.output.expanduser().resolve()
    written = 0
    try:
        output_path.parent.mkdir(parents=True, exist_ok=True)
        with output_path.open("a", encoding="utf-8", newline="\n") as writer:
            print(f"正在追加日志：{output_path}（Ctrl+C 停止）", flush=True)
            while args.total_lines == 0 or written < args.total_lines:
                batch_size = rng.randint(args.min_lines, args.max_lines)
                if args.total_lines:
                    batch_size = min(batch_size, args.total_lines - written)
                for _ in range(batch_size):
                    writer.write(build_log_line(rng, written + 1))
                    written += 1
                writer.flush()
                if args.total_lines and written >= args.total_lines:
                    break
                time.sleep(args.interval)
    except KeyboardInterrupt:
        print("\n已停止。")
    except OSError as error:
        print(f"写入失败：{error}", file=sys.stderr)
        return 1
    print(f"本次已追加 {written} 行：{output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
