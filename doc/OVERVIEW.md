## Debug Information

For `dev` builds we split debug information using `split-debuginfo="packed"`,
which seems to speed up linking time and the resulting binaries are not
distributed anyway. For `release` builds we enable `debug = "line-tables-only"`
to enable line and function information in backtraces. In the `mcrl2` tools we
do not split debug info since `cpptrace` cannot seem to find it, and this breaks
stack traces in the `C++` code.