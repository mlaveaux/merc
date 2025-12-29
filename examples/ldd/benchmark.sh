#!/bin/bash

if ! command -v "hyperfine" &> /dev/null
then
    echo "Install hyperfine by running 'cargo install hyperfine' or using another package manager first."
    exit
fi

if ! command -v "reach" &> /dev/null
then
    echo "Ensure that 'reach' can be found in path."
    exit
fi

if ! command -v "lddmc" &> /dev/null
then
    echo "WARNING: To run the lddmc benchmarks ensure that it can be found in path, otherwise will be skipped."
fi

# Perform the benchmarks for reach.
BENCHMARKS=()
for file in *.ldd
do
  BENCHMARKS+=("reach $file")
done

hyperfine --warmup 3 -i -u second --export-markdown results_reach.md "${BENCHMARKS[@]/#}"

if command -v "lddmc" &> /dev/null
then
  # Perform the benchmarks for lddmc
  BENCHMARKS=()
  for file in *.ldd
  do
    BENCHMARKS+=("lddmc -w1 -sbfs $file")
  done

  hyperfine --warmup 3 -i -u second --export-markdown results_lddmc.md "${BENCHMARKS[@]/#}"
fi