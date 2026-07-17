# Axiom access for agents

Use Axiom's hosted MCP server for interactive investigation. It authenticates
in the browser with OAuth, so an agent does not receive or print an API token.

```sh
scripts/axiom-agent.sh mcp-login
scripts/axiom-agent.sh mcp-status
```

The configured endpoint is `https://mcp.axiom.co/mcp`. Restart the agent after
the first login so its tool inventory includes Axiom. Axiom also documents an
SSE endpoint for clients that have not adopted streamable HTTP; Codex uses the
streamable HTTP endpoint above.

For automation that cannot use OAuth, create a separate, short-lived token with
only the dataset query permissions it needs. Do not give an agent organization,
delete, or administrative permissions. Keep credentials in `.env` or deployment
secrets and never pass them as command-line arguments.

This repository recognizes two roles:

- `AXIOM_API_KEY` (or legacy `AXIOM_TOKEN`) is the application ingestion key.
- `AXIOM_PROD_API_KEY` is the operator/agent query key used by the safe report.

Check both without displaying either token or response data:

```sh
scripts/axiom-agent.sh check
scripts/axiom-agent.sh report
```

`check` only reports authentication and whether dataset listing is permitted.
`report` emits aggregate latency statistics and never emits raw search queries.
An ingestion-only application key may correctly return HTTP 403 for dataset
listing; the query key should have read access to `AXIOM_DATASET`.

Official references:

- [Axiom MCP server](https://axiom.co/docs/console/intelligence/mcp-server)
- [Axiom AI agent security](https://axiom.co/docs/console/intelligence/ai-agents-overview)
- [Axiom REST API authentication](https://axiom.co/docs/restapi/introduction)
