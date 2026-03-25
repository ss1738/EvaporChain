# MCP Server for AI Agents

EvaporChain ships a native [Model Context Protocol](https://modelcontextprotocol.io) (MCP) server that lets AI agents interact with the blockchain directly. This is the first blockchain with built-in AI agent support.

The MCP server provides 15 tools, 7 resources, and 3 prompts over stdio using JSON-RPC 2.0.

## Setup

### Prerequisites

- Rust toolchain (cargo)
- A running EvaporChain node (testnet or local)

### Build

```bash
cd /path/to/EvaporChain
cargo build -p evaporchain-mcp --release
```

### Configure for Claude Desktop

Add to your Claude Desktop config (`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS):

```json
{
  "mcpServers": {
    "evaporchain": {
      "command": "cargo",
      "args": ["run", "-p", "evaporchain-mcp", "--", "--node-url", "http://37.27.1.1:8080"],
      "cwd": "/path/to/EvaporChain"
    }
  }
}
```

Or use the pre-built binary:

```json
{
  "mcpServers": {
    "evaporchain": {
      "command": "/path/to/EvaporChain/target/release/evaporchain-mcp",
      "args": ["--node-url", "https://testnet.evaporchain.com"]
    }
  }
}
```

### Configure for Claude Code

Add to `.claude/mcp-config.json` or `mcp-config.json` in the project root:

```json
{
  "mcpServers": {
    "evaporchain": {
      "command": "cargo",
      "args": ["run", "-p", "evaporchain-mcp", "--", "--node-url", "https://testnet.evaporchain.com"],
      "cwd": "/path/to/EvaporChain"
    }
  }
}
```

### Command Line Options

```
evaporchain-mcp [--node-url URL]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--node-url` | `http://37.27.1.1:8080` | EvaporChain node HTTP API URL |

## Tools (15)

AI agents can use these tools to read and write to the blockchain.

### Read Tools

| Tool | Description |
|------|-------------|
| `get_chain_status` | Block height, epoch, object counts, uptime, state root |
| `list_objects` | All active state objects with energy decay info |
| `get_object` | Single object by hex ID — energy, decay %, half-life, state |
| `list_accounts` | All accounts with balances and nonces |
| `list_ghosts` | All evaporated objects (ghost records) |
| `get_recent_blocks` | Recent blocks with transactions and state transitions |
| `get_block` | Specific block by number |
| `get_recent_events` | Chain events: evaporations, creations, grace transitions |
| `list_contracts` | All deployed smart contracts |
| `get_stats` | Aggregate statistics: created, evaporated, resurrected counts |

### Write Tools

| Tool | Description |
|------|-------------|
| `transfer` | Transfer EVAP between accounts |
| `create_object` | Create a new decaying state object |
| `refresh_object` | Deposit energy to extend object lifetime |
| `resurrect_object` | Bring back an evaporated ghost object |
| `request_faucet` | Claim 10,000 testnet EVAP (rate limited) |

### Tool Examples

An AI agent can explore the chain:

```
Agent: Let me check the chain status.
[calls get_chain_status]
→ Block 4521, Epoch 4521, 12 active objects, 5 ghosts

Agent: Which objects are close to evaporating?
[calls list_objects]
→ Object 0x0a has 3% energy remaining (half-life 10, grace expected in ~2 epochs)

Agent: I should refresh that object before it evaporates.
[calls refresh_object with object_id=10, energy_deposit=5000]
→ Object refreshed successfully
```

## Resources (7)

Resources provide live blockchain data that agents can read as context.

| URI | Description |
|-----|-------------|
| `evaporchain://status` | Live chain status (JSON) |
| `evaporchain://objects` | All active objects with energy levels |
| `evaporchain://ghosts` | All ghost records |
| `evaporchain://accounts` | Account balances |
| `evaporchain://blocks` | Last 50 blocks |
| `evaporchain://events` | Recent chain events |
| `evaporchain://stats` | Aggregate statistics |

Resources are read-only and always return the latest state from the node.

## Prompts (3)

Prompts are guided workflows that give AI agents structured context for common tasks.

### explore_chain

Provides the agent with current chain status, active objects, and recent events, then asks for analysis.

The agent receives:
- Full chain status JSON
- All active state objects with energy levels
- Last 10 events

And is asked to analyze:
1. Overall chain health
2. Objects close to evaporating
3. Interesting event patterns
4. Objects that should be refreshed

### create_and_watch

Guides the agent through creating a state object and watching it decay. Accepts optional `energy` and `half_life` parameters.

```json
{
  "name": "create_and_watch",
  "arguments": {
    "energy": "100",
    "half_life": "5"
  }
}
```

The agent receives instructions to:
1. Create an object with the specified parameters
2. Check its initial state
3. Wait and observe decay
4. Explain thermodynamic state decay

### chain_health_report

Provides comprehensive chain data for a structured health report. The agent receives:
- Chain status
- Aggregate statistics
- All active objects
- All ghost records
- Last 50 events

And produces a report covering block production, state lifecycle, evaporation metrics, energy distribution, network health, and recommendations.

## Protocol

The MCP server communicates over stdio using JSON-RPC 2.0 as specified by the [MCP specification](https://spec.modelcontextprotocol.io).

### Capabilities

```json
{
  "capabilities": {
    "tools": {},
    "resources": {},
    "prompts": {}
  },
  "serverInfo": {
    "name": "evaporchain-mcp",
    "version": "0.1.0"
  }
}
```

### Methods

| Method | Description |
|--------|-------------|
| `initialize` | Handshake, returns capabilities |
| `tools/list` | List available tools |
| `tools/call` | Execute a tool |
| `resources/list` | List available resources |
| `resources/read` | Read a resource |
| `prompts/list` | List available prompts |
| `prompts/get` | Get a prompt with context |

## Architecture

```
AI Agent (Claude, etc.)
    │
    │ stdio (JSON-RPC 2.0)
    ▼
evaporchain-mcp
    │
    │ HTTP API calls
    ▼
EvaporChain Node (testnet.evaporchain.com)
```

The MCP server is a thin proxy — it translates MCP tool calls into HTTP API calls against the node. No blockchain state is stored in the MCP server.

## Use Cases

### Autonomous State Management

An AI agent can monitor objects approaching evaporation and decide whether to refresh them based on importance:

```
Agent observes: "UserData-42 has 5% energy remaining"
Agent decides: "This contains important user preferences, refreshing"
Agent calls: refresh_object(object_id=42, energy_deposit=10000)
```

### Chain Health Monitoring

Use the `chain_health_report` prompt to have an agent produce regular health reports, identifying trends and anomalies in evaporation rates.

### Interactive Demos

The `create_and_watch` prompt lets agents give live demonstrations of thermodynamic state decay to users who are learning about EvaporChain.

### Automated Testing

AI agents can create objects with known decay parameters, wait for specific epochs, and verify that the decay formula produces expected results — automated property testing of the thermodynamic model.
