#!/bin/bash
cd /Volumes/samsung_t9/kodegen-workspace/packages/kodegen-tools-terminal
echo "Starting test at $(date)"
timeout 10 bash -c 'RUST_LOG=info cargo run --example ls > /tmp/terminal_test.log 2>&1'
EXIT_CODE=$?
echo "Exit code: $EXIT_CODE at $(date)"
if [ $EXIT_CODE -eq 124 ]; then
    echo "TIMEOUT - Process hung and was killed"
    echo "Last 50 lines of output:"
    tail -50 /tmp/terminal_test.log
elif [ $EXIT_CODE -eq 0 ]; then
    echo "SUCCESS - Process exited cleanly"
    echo "Last 20 lines of output:"
    tail -20 /tmp/terminal_test.log
else
    echo "FAILED - Process exited with code $EXIT_CODE"
    echo "Last 50 lines of output:"
    tail -50 /tmp/terminal_test.log
fi
