FROM ubuntu:latest

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

ENV DEBIAN_FRONTEND=noninteractive
ENV PATH="/home/vscode/.cargo/bin:/home/vscode/.local/bin:/usr/local/bin:${PATH}"

ARG USERNAME=vscode
ARG USER_UID=1001
ARG USER_GID=1001

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        build-essential \
        ca-certificates \
        curl \
        docker.io \
        git \
        gnupg \
        jq \
        libssl-dev \
        openssh-client \
        pkg-config \
        ripgrep \
        sudo \
        xz-utils \
        zsh \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --gid "${USER_GID}" "${USERNAME}" \
    && useradd --uid "${USER_UID}" --gid "${USER_GID}" -m -s /bin/bash "${USERNAME}" \
    && usermod -aG sudo,docker,root "${USERNAME}" \
    && echo "${USERNAME} ALL=(ALL) NOPASSWD:ALL" >/etc/sudoers.d/99-"${USERNAME}" \
    && chmod 0440 /etc/sudoers.d/99-"${USERNAME}"

RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get update \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

USER ${USERNAME}
WORKDIR /workspaces/magi

RUN curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain stable \
    && rustup component add rustfmt clippy

RUN curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_NON_INTERACTIVE=1 sh
RUN curl -fsSL https://claude.ai/install.sh | bash
RUN curl -fsSL https://raw.githubusercontent.com/kent8192/magi-system/main/install.sh | bash

CMD ["/bin/bash"]
