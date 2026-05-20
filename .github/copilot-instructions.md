# GitHub Copilot Instructions — dr-house-rs

## Differential Diagnosis for Code

Dr House, TTD (Time Travel Debugger) is a VS Code extension that applies 
differential diagnosis techniques to debugging and code analysis. Rather than 
running arbitrary cargo commands, this project uses structured tools and 
record-and-replay debugging to identify and fix issues.

When working in this project, focus on:

1. **Diagnostic Tools** — Use analysis and inspection tools to narrow down root causes
2. **Time Travel Debugging** — Leverage record-and-replay capabilities to step backwards through execution
3. **Differential Diagnosis** — Test hypotheses systematically to eliminate possibilities
4. **Structured Output** — All tools provide structured, queryable results

### Extension Development

This VS Code extension is built with TypeScript and bundles debugging diagnostics
as first-class features. Focus on:

- Clean separation between diagnostics engine and UI
- Structured output suitable for both human and machine consumption
- Integration with VS Code's Debug Protocol when appropriate
- Test-driven development for diagnostic accuracy

### Development Workflow

- Use the build and test tools provided
- Maintain comprehensive test coverage for diagnostic accuracy
- Document assumptions in diagnostic implementations
- Keep the extension lightweight and performant
