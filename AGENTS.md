# AIRP Agent Memory

## Local Build Environment

- This Windows workspace keeps build tooling on `D:`. Do not install Rust, Cargo, Node, npm globals, MSYS2, caches, or generated build dependencies under `C:`.
- Confirmed local toolchain roots:
  - `RUSTUP_HOME=D:\.rustup`
  - `CARGO_HOME=D:\.cargo`
  - Rust shims: `D:\.cargo\bin`
  - MSYS2/GNU linker path: `D:\msys64\mingw64\bin`
  - Node.js: `D:\nodejs`
  - npm global prefix/cache area: `D:\npm-global`
- Before local Rust builds/tests in PowerShell, set:
  ```powershell
  $env:RUSTUP_HOME = "D:\.rustup"
  $env:CARGO_HOME = "D:\.cargo"
  $env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH
  ```
- Use the default repo target directory `D:\AIRP-Dev\target` unless a task explicitly requires otherwise.
- If a command tries to populate `C:\Users\<user>\.cargo`, `C:\Users\<user>\.rustup`, or npm cache/global data under `C:`, stop and redirect it to the D-drive locations above.
