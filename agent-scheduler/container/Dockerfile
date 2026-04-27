FROM node:22-bookworm-slim AS base

# Install system deps
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl git ca-certificates jq python3 \
    && rm -rf /var/lib/apt/lists/*

# Install GitHub CLI
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
      | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg && \
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
      > /etc/apt/sources.list.d/github-cli.list && \
    apt-get update && apt-get install -y gh && rm -rf /var/lib/apt/lists/*

# Install Codex CLI
RUN npm install -g @openai/codex

COPY run-task.sh /usr/local/bin/run-task.sh
RUN chmod +x /usr/local/bin/run-task.sh

WORKDIR /workspace
ENTRYPOINT ["/usr/local/bin/run-task.sh"]
