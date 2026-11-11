# Fake upstream for the parity harness. Stack-agnostic: every stack under test
# uses the same image. No npm dependencies; Node 24 runs the .ts files with
# type stripping.
ARG NODE_IMAGE=node:24.22.1-trixie-slim
FROM ${NODE_IMAGE}

RUN apt-get update \
  && apt-get install -y --no-install-recommends openssl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /fake-upstream
COPY *.ts hosts.txt gen-certs.sh ./
RUN chmod +x gen-certs.sh

ENV TLS_DIR=/parity-tls \
  HOSTS_FILE=/fake-upstream/hosts.txt \
  FAKE_UPSTREAM_IP=1.1.1.1 \
  NODE_ENV=production

EXPOSE 53/udp 53/tcp 80 443 4010
HEALTHCHECK --interval=2s --timeout=3s --retries=30 \
  CMD ["node", "-e", "fetch('http://127.0.0.1:4010/__admin/health').then(r=>process.exit(r.ok?0:1),()=>process.exit(1))"]
CMD ["sh", "-c", "./gen-certs.sh && exec node server.ts"]
