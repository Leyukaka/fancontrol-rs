# PawnIO modules (vendored)

Official signed builds from [namazso/PawnIO.Modules](https://github.com/namazso/PawnIO.Modules/releases) **v0.2.9**.

| File | Purpose |
|------|---------|
| `LpcIO.bin` | Super I/O / LPC port access (motherboard fans & sensors) |
| `Echo.bin` | Smoke-test module |
| `AMDFamily17.bin` | AMD Zen (family 17h–1Ah) MSR — CPU package power (`ioctl_read_msr`), read-only |
| `IntelMSR.bin` | Intel RAPL MSR — package energy / power info / DRAM energy, read-only |
| `COPYING` | LGPL-2.1 license from upstream |

Do not rebuild these from source unless you know how to sign them for PawnIO.
