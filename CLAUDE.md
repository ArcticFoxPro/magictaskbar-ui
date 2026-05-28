# CLAUDE.md

## Project Overview

MagicTaskBar - Customizable Windows taskbar built with Rust + TypeScript/React + Tauri.

## Quick Start

```bash
npm install && npm run dev
```

## Common Commands

- `npm run dev` - Development mode
- `npm run build:ui` - Build UI
- `cargo build` - Build Rust backend
- `deno lint` / `deno fmt` - Code quality

## Architecture

**Monorepo Structure:**
- `libs/` - Shared libraries (core, IPC)
- `src/background/` - Rust backend
- `src/ui/` - Frontend app--taskbar

**Tech Stack:**
- Backend: Rust + Tauri + Windows API
- Frontend: React/Preact + Redux + Ant Design
- Build: esbuild + Cargo + Deno
