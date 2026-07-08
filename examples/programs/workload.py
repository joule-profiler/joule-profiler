#!/usr/bin/env python3
"""
Simple workload for testing joule-profiler phase detection.

Usage:
    joule-profiler profile -- python3 workload.py
"""

import argparse

def find_primes(limit):
    """Find all prime numbers up to limit."""
    primes = []
    for num in range(2, limit):
        is_prime = True
        for i in range(2, int(num ** 0.5) + 1):
            if num % i == 0:
                is_prime = False
                break
        if is_prime:
            primes.append(num)
    return primes


def main():
    parser = argparse.ArgumentParser(description='Prime number calculation')
    parser.add_argument('-n', '--limit', type=int, default=50000,
                        help='Find primes up to this number (default: 50000)')

    args = parser.parse_args()

    print("Starting prime calculation...")
    print("__WORK_START__", flush=True)

    primes = find_primes(args.limit)

    print("__WORK_END__", flush=True)
    print(f"Found {len(primes)} prime numbers up to {args.limit}")


if __name__ == '__main__':
    main()