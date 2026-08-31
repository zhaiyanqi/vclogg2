#!/usr/bin/env python3

import argparse
import os
import random
import secrets
from pathlib import Path
from typing import BinaryIO, Dict, Iterable, List, Tuple


MEBIBYTE = 1024 * 1024
SIZE_OPTIONS: Dict[str, int] = {
    "50M": 50 * MEBIBYTE,
    "100M": 100 * MEBIBYTE,
    "500M": 500 * MEBIBYTE,
}
OVERFLOW_MIN_CHARS = 800
OVERFLOW_MAX_CHARS = 2400

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
OVERFLOW_FRAGMENTS = (
    "account=enterprise-north ",
    "action=refresh-materialized-view ",
    "component=distributed-request-coordinator ",
    "detail=synthetic-overflow-content-for-horizontal-scroll-testing ",
    "feature=virtualized-log-table ",
    "header=x-vclogg-synthetic-test-data ",
    "operation=batch-reconciliation ",
    "partition=ap-southeast-1-primary ",
    "resource=/api/v2/organizations/synthetic/workspaces/performance/events ",
    "result=completed-with-recoverable-warning ",
)


def parse_size(value: str) -> str:
    normalized = value.upper()
    if normalized not in SIZE_OPTIONS:
        choices = ", ".join(SIZE_OPTIONS)
        raise argparse.ArgumentTypeError(f"档位必须是 {choices} 之一")
    return normalized


def parse_rate(value: str) -> float:
    try:
        rate = float(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("超长行比例必须是数字") from error
    if not 0.0 < rate <= 1.0:
        raise argparse.ArgumentTypeError("超长行比例必须大于 0 且不超过 1")
    return rate


def unique_sizes(values: Iterable[str]) -> List[str]:
    result: List[str] = []
    for value in values:
        if value not in result:
            result.append(value)
    return result


def random_hex(rng: random.Random, width: int) -> str:
    return f"{rng.getrandbits(width * 4):0{width}x}"


def build_overflow_payload(rng: random.Random) -> str:
    target_chars = rng.randint(OVERFLOW_MIN_CHARS, OVERFLOW_MAX_CHARS)
    fragments: List[str] = []
    length = 0
    while length < target_chars:
        fragment = rng.choice(OVERFLOW_FRAGMENTS)
        fragments.append(fragment)
        length += len(fragment)
    return "".join(fragments)[:target_chars]


def build_log_line(
    rng: random.Random,
    line_number: int,
    long_line_rate: float,
    guaranteed_overflow_line: int,
) -> Tuple[bytes, bool]:
    is_overflow = (
        line_number == guaranteed_overflow_line or rng.random() < long_line_rate
    )
    day = rng.randint(1, 28)
    hour = rng.randrange(24)
    minute = rng.randrange(60)
    second = rng.randrange(60)
    millisecond = rng.randrange(1000)
    level = rng.choice(LEVELS)
    service = rng.choice(SERVICES)
    status = rng.choice((200, 200, 201, 204, 400, 404, 409, 429, 500, 503))
    latency_ms = rng.randrange(1, 5001)
    prefix = (
        f"2026-08-{day:02}T{hour:02}:{minute:02}:{second:02}.{millisecond:03}Z "
        f"{level:<5} service={service} line={line_number} "
        f"request_id={random_hex(rng, 16)} trace_id={random_hex(rng, 32)} "
        f"client=10.{rng.randrange(256)}.{rng.randrange(256)}.{rng.randrange(1, 255)} "
        f"status={status} latency_ms={latency_ms}"
    )

    if is_overflow:
        message = (
            f" overflow=true event=wide-row payload={build_overflow_payload(rng)}"
        )
    else:
        message = f" overflow=false event={rng.choice(EVENTS)}"
    return f"{prefix}{message}\n".encode("ascii"), is_overflow


def write_tail(writer: BinaryIO, remaining: int) -> None:
    if remaining == 1:
        writer.write(b"\n")
    elif remaining == 2:
        writer.write(b"#\n")
    else:
        writer.write(b"#" + (b"-" * (remaining - 2)) + b"\n")


def generate_file(
    output_path: Path,
    target_bytes: int,
    base_seed: int,
    long_line_rate: float,
) -> Tuple[int, int]:
    rng = random.Random(base_seed ^ target_bytes)
    guaranteed_overflow_line = rng.randint(2, 25)
    temporary_path = output_path.with_name(
        f".{output_path.name}.tmp-{os.getpid()}-{random_hex(rng, 8)}"
    )
    written = 0
    line_count = 0
    overflow_count = 0

    try:
        with temporary_path.open("xb", buffering=1024 * 1024) as writer:
            while written < target_bytes:
                next_line_number = line_count + 1
                line, is_overflow = build_log_line(
                    rng,
                    next_line_number,
                    long_line_rate,
                    guaranteed_overflow_line,
                )
                remaining = target_bytes - written
                if len(line) <= remaining:
                    writer.write(line)
                    overflow_count += int(is_overflow)
                    written += len(line)
                else:
                    write_tail(writer, remaining)
                    written += remaining
                line_count = next_line_number
        temporary_path.replace(output_path)
    finally:
        try:
            temporary_path.unlink()
        except FileNotFoundError:
            pass

    return line_count, overflow_count


def build_parser() -> argparse.ArgumentParser:
    repository_root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(
        description=(
            "生成 VCLogg2 大文件测试日志；默认生成 50M、100M、500M 三档。"
        )
    )
    parser.add_argument(
        "sizes",
        nargs="*",
        type=parse_size,
        metavar="SIZE",
        help="要生成的档位：50M、100M 或 500M；省略时生成全部档位",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=repository_root / "target" / "test-data",
        help="输出目录（默认：仓库下的 target/test-data）",
    )
    parser.add_argument(
        "--seed",
        type=int,
        help="随机种子；指定后可复现相同档位的数据",
    )
    parser.add_argument(
        "--long-line-rate",
        type=parse_rate,
        default=0.08,
        metavar="RATE",
        help="随机超长行比例，范围 (0, 1]（默认：0.08）",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="覆盖已经存在的同名测试文件",
    )
    return parser


def main() -> None:
    args = build_parser().parse_args()
    sizes = unique_sizes(args.sizes or SIZE_OPTIONS.keys())
    output_directory = args.output_dir.expanduser().resolve()
    output_directory.mkdir(parents=True, exist_ok=True)
    destinations = {
        size: output_directory / f"vclogg-test-{size.lower()}.log" for size in sizes
    }
    existing = [path for path in destinations.values() if path.exists()]
    if existing and not args.force:
        paths = "\n".join(f"  {path}" for path in existing)
        raise SystemExit(f"以下文件已存在；如需覆盖请添加 --force：\n{paths}")

    seed: int = args.seed if args.seed is not None else secrets.randbits(64)
    print(f"随机种子：{seed}")
    for size in sizes:
        output_path = destinations[size]
        target_bytes = SIZE_OPTIONS[size]
        print(f"正在生成 {size}：{output_path}", flush=True)
        line_count, overflow_count = generate_file(
            output_path,
            target_bytes,
            seed,
            args.long_line_rate,
        )
        actual_bytes = output_path.stat().st_size
        if actual_bytes != target_bytes:
            raise RuntimeError(
                f"生成后的大小不正确：期望 {target_bytes}，实际 {actual_bytes}"
            )
        print(
            f"完成 {size}：{actual_bytes} 字节，{line_count} 行，"
            f"其中 {overflow_count} 行超过常见屏幕宽度"
        )


if __name__ == "__main__":
    main()
