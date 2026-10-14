# Lessons

## Ask before changing the stack of an existing project

- **Mistake:** The repo is a Rust app. I built the macOS version in Swift without asking. The user had to correct it, and the Swift work was wasted.
- **Rule:** When a request could be met in a different language, framework or toolchain from the project's own, ask first. Do not decide it silently. Name the options and the cost of each, then wait for an answer.
- **Rule:** Before writing code for a new platform, check which parts of the existing code can be reused. Reuse them rather than rewrite them.
- **Rule:** Installing a toolchain needs the user's explicit go-ahead. Get it before running the installer.
