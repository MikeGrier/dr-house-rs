# Dr House, TTD

A VS Code extension for expert differential diagnosis and time travel debugging.

## Overview

Dr House, TTD (Time Travel Debugger) brings intelligent debugging techniques to Visual Studio Code. Inspired by the TV character's ability to solve complex medical mysteries through differential diagnosis, this extension applies similar diagnostic techniques to code analysis and debugging.

### Key Features

- **Differential Diagnosis** — Systematically narrow down root causes through structured hypothesis testing
- **Time Travel Debugging** — Step backwards through execution to understand what led to an issue
- **Diagnostic Tools** — Structured analysis tools designed for machine and human consumption
- **Clean UI** — Integrates seamlessly with VS Code's debugging experience

## Development

This is a multi-workspace Rust + TypeScript project:

```
dr-house-rs/
├── crates/
│   └── dr-house-extension/          # VS Code extension (TypeScript)
├── Cargo.toml                        # Workspace configuration
├── package.json                      # Root npm configuration
└── .github/
    ├── workflows/                    # CI/CD pipelines
    └── copilot-instructions.md       # GitHub Copilot guidance
```

### Prerequisites

- Node.js 20+
- Rust 1.70+ (for any Rust components)
- VS Code 1.80+ (for development/testing)

### Building

```bash
npm install
npm run build
```

### Testing

```bash
npm test
```

### Publishing

The extension publishes to the [Visual Studio Marketplace](https://marketplace.visualstudio.com/) via GitHub Actions on tagged releases.

## License

MIT
