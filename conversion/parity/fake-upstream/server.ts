// Fake third-party upstream for the parity harness. One process serves:
//   DNS  udp+tcp :53   every A query -> FAKE_UPSTREAM_IP (1.1.1.1)
//   HTTPS :443         test-CA leaf cert, routed by Host (includes DoH on 1.1.1.1)
//   HTTP  :80          same router
//   admin :4010        see admin.ts
// Runs with Node 24 type stripping: `node server.ts`. No npm dependencies.

import http from "node:http"
import https from "node:https"
import { createAdminHandler } from "./admin.ts"
import { FAKE_UPSTREAM_IP, startDnsServers } from "./dns.ts"
import { createUpstreamHandler } from "./router.ts"
import { createTlsOptions } from "./tls.ts"

const tlsDir = process.env.TLS_DIR ?? "/parity-tls"
const hostsFile = process.env.HOSTS_FILE ?? new URL("./hosts.txt", import.meta.url).pathname
const port = (name: string, fallback: number): number =>
  Number.parseInt(process.env[name] ?? String(fallback), 10)

startDnsServers(port("DNS_PORT", 53))

https
  .createServer(createTlsOptions({ tlsDir, hostsFile }), createUpstreamHandler("https"))
  .on("tlsClientError", () => {
    // Expected for unknown SNI names and clients that reject the test CA.
  })
  .listen(port("HTTPS_PORT", 443), "0.0.0.0")

http.createServer(createUpstreamHandler("http")).listen(port("HTTP_PORT", 80), "0.0.0.0")

http
  .createServer(createAdminHandler({ caFile: `${tlsDir}/ca.crt` }))
  .listen(port("ADMIN_PORT", 4010), "0.0.0.0", () => {
    console.log(
      JSON.stringify({ msg: "fake upstream ready", ip: FAKE_UPSTREAM_IP, tlsDir }),
    )
  })
