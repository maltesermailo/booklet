# KGDB setup

Kernel debugging over serial needs three pieces: a UART reachable from outside, `kgdboc` pointed at it, and a debugger on the host speaking the GDB remote protocol. On the [[Port log]] device the console UART is only exposed on the debug connector.

## Kernel configuration 3

Enable the debugger and keep the watchdog from biting while execution is halted — see [[CFI violations]].

```
CONFIG_KGDB=y
CONFIG_KGDB_SERIAL_CONSOLE=y
kgdboc=ttySAC0,115200 kgdbwait
```

> A halted kernel tells the truth; a printk tells a story.

Open question: trapping the Mali DVFS path from [[Wayland and KMS]] before the NULL deref.
