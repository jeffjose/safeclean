# safeclean

Clean up build artifacts and dependency caches.

```bash
safeclean                # scan current dir, interactive selection
safeclean ~/projects     # scan specific path
safeclean --rust         # only Rust target/ dirs
safeclean --node         # only node_modules/
safeclean -n             # dry run
safeclean -y             # skip confirmation
```

Supports: Rust, Node.js, Python, Java/Maven, Gradle, .NET, Next.js, Nuxt.js
