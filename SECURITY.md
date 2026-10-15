# Security policy

## Reporting a vulnerability

Please do **not** open a public issue for security problems. Use GitHub's
[private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability)
on this repository instead, with a description, the affected version and, if possible, steps to
reproduce. You should get an answer within a week.

## Deployment advice

- Keep the login enabled unless an authenticating reverse proxy sits in front of Rustload.
- Serve it over HTTPS when it is reachable from the internet, and set `RUSTLOAD_SECURE_COOKIES=true`.
- Treat the API key like a password; regenerate it in *Settings › Integrations* if it leaks.
- Click'n'Load accepts unauthenticated requests by design (that is how the protocol works): only
  enable it when you use it, and do not expose port 9666 to the internet.
- Hooks run shell commands with the permissions of the Rustload process: only configure commands
  you trust.
