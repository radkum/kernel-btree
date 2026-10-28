# Security policy

## Supported versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | yes       |

## Reporting a vulnerability

Please report security issues privately via
[GitHub Security Advisories](https://github.com/radkum/kernel-btree/security/advisories/new).
Do not open a public issue for a vulnerability.

You can expect an acknowledgement within a few days. Please include the crate version,
target, and a minimal reproducer if possible.

## Scope

The crate contains `unsafe` code for raw node memory management and iterators. Memory-safety
bugs, undefined behaviour, double frees and use-after-free are security issues. Incorrect
results caused by a broken `Ord` implementation are documented behaviour, not a vulnerability.
