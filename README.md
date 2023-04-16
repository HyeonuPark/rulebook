Rulebook
==========

Multiplayer game framework with isomorphic WebAssembly.

# Features

- Deterministic, synchronous, linear game logic compiled to WASM.
- Both server and clients runs same logic with server-routed IO events.
- Game logic defines which information is allowed for each clients.

# Components

- Shared game server which runs multiple kinds of games simultaneously.
- In-wasm library to support writing game logic.
- CLI game client for testing and development.

# Todo

- Limit WASM runtime resources to support untrusted user-provided games.
- Replay based connection recovery.
