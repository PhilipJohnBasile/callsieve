# CallSieve Product Brief

## One-liner

CallSieve is the local codebase filter for AI coding agents.

## Problem

Agentic coding tools waste massive context and cost rediscovering codebases through repeated grep, ripgrep, file reads, directory scans, and duplicate exploration.

This makes coding agents:
- slower
- more expensive
- less reliable
- worse on large repos
- worse with smaller local models

## Insight

The best way to improve coding agents is not only bigger context. It is better context selection.

CallSieve is sparse attention for the codebase before the prompt exists.

Sparse attention helps a model spend less compute on irrelevant prompt tokens. CallSieve helps an agent avoid putting irrelevant repo tokens into the prompt in the first place.

The distinction matters:

- CallSieve solves finding information.
- Sparse attention solves reasoning over information already found.
- If the relevant function is not in the prompt, sparse attention cannot find it.

Agents should not ask:

```bash
rg "UserService"
```

They should ask:

```text
show_symbol(UserService)
find_callers(UserService)
find_tests(UserService)
explain_module(auth)
```

## Retrieval Model

CallSieve should use a coarse-to-fine retrieval pipeline:

```text
User question
  -> repo index
  -> top-k files, symbols, tests, and call neighborhoods
  -> relevant snippets
  -> compact context for the coding agent
```

The first implementation should use deterministic ranking:

- exact symbol matches
- path and filename matches
- import proximity
- test proximity
- keyword overlap

Later versions can add semantic reranking, embeddings, richer call graphs, and long-term agent memory.

## Initial Product

A local CLI that indexes a repo and answers compact codebase questions.

## MVP Commands

```bash
callsieve index .
callsieve symbols .
callsieve symbol . UserService
callsieve query . "where is login handled?"
callsieve stats .
```

## Future Product

CallSieve becomes:

- MCP server
- codebase graph
- symbol memory
- call path retriever
- test impact mapper
- blast radius analyzer
- agent context runtime

## Core Differentiator

Most tools help agents generate code.

CallSieve helps agents stop wasting tokens before they generate code.
