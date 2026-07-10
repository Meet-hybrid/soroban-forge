# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

Please report security vulnerabilities by one of the following methods:

- **GitHub Security Advisory**: Use the "Security" tab on the repository.
- **Email**: Send details to **security@teachlink.org**.

Include:
- Description of the vulnerability.
- Steps to reproduce.
- Affected versions.
- Suggested fix if available.

We will acknowledge receipt within 72 hours and provide a detailed response within 7 days.

**Please do NOT open public GitHub issues for security vulnerabilities.**

## Security Practices

- All smart contracts are subject to formal verification where feasible.
- External audits are performed before major releases.
- Dependencies are audited automatically via CI using `cargo audit`.
- Critical and high severity vulnerabilities are patched within 14 days.
- Low/medium issues are scheduled for the next minor release.

## Disclosure Policy

- We follow coordinated disclosure.
- Fixes are released as patch versions.
- Public disclosure occurs after a fix is available.

Contact: **security@teachlink.org**
