import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { after, before, describe, test } from "node:test";
import * as pulumi from "@pulumi/pulumi";
import {
  requireImmutableImage,
  validateHttpsOrigin,
  validateWrappingKeyVersions,
} from "./policy";

interface ResourceRecord {
  type: string;
  name: string;
  inputs: Record<string, unknown>;
}

const resources: ResourceRecord[] = [];
const previousConfig = process.env.PULUMI_CONFIG;
const previousSecretKeys = process.env.PULUMI_CONFIG_SECRET_KEYS;
const previousHomeAssistantUrl = process.env.HOME_ASSISTANT_URL;
const previousHomeAssistantToken = process.env.HOME_ASSISTANT_TOKEN;
let program: typeof import("./index");

before(async () => {
  process.env.PULUMI_CONFIG = JSON.stringify({
    "smarthome-mcp:namespace": "smarthome-mcp-test",
    "smarthome-mcp:hostname": "smarthome-mcp.example.test",
    "smarthome-mcp:displayName": "Smarthome MCP Test",
    "smarthome-mcp:slug": "smarthome-mcp-test",
    "smarthome-mcp:image":
      "registry.example.test/smarthome-mcp@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "smarthome-mcp:authentikBaseUrl": "https://auth.example.test",
    "smarthome-mcp:protectData": "true",
    "smarthome-mcp:storageClass": "test-storage",
    "smarthome-mcp:databaseStorageSize": "2Gi",
    "smarthome-mcp:backupStorageClass": "test-bucket",
    "smarthome-mcp:backupEndpoint": "https://s3.example.test",
    "smarthome-mcp:backupRetention": "7d",
    "smarthome-mcp:backupSchedule": "0 30 1 * * *",
    "smarthome-mcp:mcpOAuthAccessTokenTtl": "300",
    "smarthome-mcp:mcpOAuthRefreshTokenTtl": "86400",
    "smarthome-mcp:mcpOAuthRefreshFamilyTtl": "2592000",
    "smarthome-mcp:mcpOAuthCodeTtl": "300",
    "smarthome-mcp:mcpOAuthCimdTrustedPrivateOrigins":
      '["https://kuri.internal.example"]',
    "smarthome-mcp:mcpOAuthWrappingKeyVersions": '["v1"]',
    "smarthome-mcp:mcpOAuthActiveWrappingKeyVersion": "v1",
  });
  process.env.PULUMI_CONFIG_SECRET_KEYS = JSON.stringify([]);
  process.env.HOME_ASSISTANT_URL = "https://home-assistant.example.test";
  process.env.HOME_ASSISTANT_TOKEN = "test-home-assistant-token";

  pulumi.runtime.setMocks(
    {
      newResource: (args) => {
        resources.push({
          type: args.type,
          name: args.name,
          inputs: args.inputs,
        });
        const outputs: Record<string, unknown> = { ...args.inputs };
        if (args.type === "random:index/randomBytes:RandomBytes") {
          outputs.base64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        }
        if (args.type === "random:index/randomPassword:RandomPassword") {
          outputs.result = "test-client-secret";
        }
        if (args.type === "tls:index/privateKey:PrivateKey") {
          outputs.privateKeyPem = "test-private-key";
        }
        if (args.type === "tls:index/selfSignedCert:SelfSignedCert") {
          outputs.certPem = "test-certificate";
        }
        if (args.type === "pulumi:index:Stash") {
          outputs.output = args.inputs.input;
        }
        if (
          args.type === "authentik:index/certificateKeyPair:CertificateKeyPair"
        ) {
          outputs.id = "signing-key";
        }
        if (args.name === "smarthome-mcp-backups-generated-config") {
          outputs.data = { BUCKET_NAME: "test-backup-bucket" };
        }
        if (args.name === "smarthome-mcp-postgres-generated-app") {
          outputs.data = {
            username: Buffer.from("app").toString("base64"),
            password: Buffer.from("password").toString("base64"),
          };
        }
        return { id: `${args.name}-id`, state: outputs };
      },
      call: (args) => ({
        ...args.inputs,
        id: "lookup-id",
        propertyMappingProviderScopeId: "mapping-id",
        data: {
          BUCKET_NAME: "test-backup-bucket",
          username: Buffer.from("app").toString("base64"),
          password: Buffer.from("password").toString("base64"),
        },
      }),
    },
    "smarthome-mcp",
    "test",
    false,
  );
  program = await import("./index");
  await pulumi.runtime.disconnect();
});

after(() => {
  restoreEnv("PULUMI_CONFIG", previousConfig);
  restoreEnv("PULUMI_CONFIG_SECRET_KEYS", previousSecretKeys);
  restoreEnv("HOME_ASSISTANT_URL", previousHomeAssistantUrl);
  restoreEnv("HOME_ASSISTANT_TOKEN", previousHomeAssistantToken);
});

describe("configuration policy", () => {
  test("accepts only immutable sha256 image references", () => {
    const digest = `registry.example.test/app@sha256:${"b".repeat(64)}`;
    assert.equal(requireImmutableImage(digest), digest);
    assert.throws(() => requireImmutableImage("registry.example.test/app:main"));
    assert.throws(() => requireImmutableImage("registry.example.test/app@sha256:abc"));
  });

  test("validates HTTPS origins and versioned wrapping keys", () => {
    assert.equal(
      validateHttpsOrigin("https://auth.example.test/", "auth"),
      "https://auth.example.test",
    );
    assert.throws(() => validateHttpsOrigin("http://auth.example.test", "auth"));
    assert.throws(() => validateHttpsOrigin("https://auth.example.test/path", "auth"));
    assert.doesNotThrow(() => validateWrappingKeyVersions(["v1"], "v1"));
    assert.throws(() => validateWrappingKeyVersions([], "v1"));
    assert.throws(() => validateWrappingKeyVersions(["v1", "v1"], "v1"));
    assert.throws(() => validateWrappingKeyVersions(["v1"], "v2"));
  });

  test("defines targets without images or deployment secrets", () => {
    const preview = stackFile("preview");
    const production = stackFile("prod");
    assert.match(preview, /^\s*smarthome-mcp:namespace: smarthome-mcp-preview$/m);
    assert.match(
      preview,
      /^\s*smarthome-mcp:hostname: preview-smarthome-mcp\.holdenitdown\.net$/m,
    );
    assert.match(production, /^\s*smarthome-mcp:namespace: smarthome-mcp$/m);
    assert.match(
      production,
      /^\s*smarthome-mcp:hostname: smarthome-mcp\.holdenitdown\.net$/m,
    );
    assert.match(preview, /^\s*smarthome-mcp:protectData: (?:"false"|false)$/m);
    assert.match(production, /^\s*smarthome-mcp:protectData: (?:"true"|true)$/m);
    for (const stack of [preview, production]) {
      assert.match(stack, /^\s*kubernetes:context: pantheon$/m);
      assert.doesNotMatch(stack, /^\s*smarthome-mcp:image:/m);
      assert.doesNotMatch(
        stack,
        /(?:password|secret|privateKey|accessKeyId|HOME_ASSISTANT_TOKEN)\s*:/i,
      );
      assert.match(
        stack,
        /^\s*smarthome-mcp:mcpOAuthWrappingKeyVersions: \[v1\]$/m,
      );
    }
  });

  test("stashes the process Home Assistant token before Secret use", () => {
    const stashes = resources.filter(
      (candidate) => candidate.type === "pulumi:index:Stash",
    );
    assert.equal(stashes.length, 2);
    const url = stashes.find((stash) => stash.name === "home-assistant-url");
    const token = stashes.find((stash) => stash.name === "home-assistant-token");
    assert.ok(url);
    assert.ok(token);
    const urlInput = url.inputs.input as Record<string, unknown>;
    const tokenInput = token.inputs.input as Record<string, unknown>;
    assert.equal(urlInput.value, "https://home-assistant.example.test");
    assert.equal(tokenInput.value, "test-home-assistant-token");
    assert.equal(Object.keys(urlInput).length, 2);
    assert.equal(Object.keys(tokenInput).length, 2);
  });
});

describe("standalone resource topology", () => {
  test("creates the namespace and protected CNPG database backups", () => {
    assert.equal(
      resource("kubernetes:core/v1:Namespace", "smarthome-mcp-namespace").inputs
        .metadata != null,
      true,
    );
    const bucket = resourceByName("smarthome-mcp-backups-bucket");
    assert.equal(bucket.inputs.kind, "ObjectBucketClaim");
    assert.equal((bucket.inputs.spec as any).storageClassName, "test-bucket");

    const cluster = resourceByName("smarthome-mcp-postgres");
    assert.equal(cluster.inputs.kind, "Cluster");
    const clusterSpec = cluster.inputs.spec as any;
    assert.equal(clusterSpec.storage.storageClass, "test-storage");
    assert.equal(clusterSpec.storage.size, "2Gi");
    assert.equal(clusterSpec.backup.retentionPolicy, "7d");
    assert.equal(
      clusterSpec.backup.barmanObjectStore.destinationPath,
      "s3://test-backup-bucket/database",
    );
    assert.deepEqual(clusterSpec.backup.barmanObjectStore.s3Credentials, {
      accessKeyId: {
        name: "smarthome-mcp-backups",
        key: "AWS_ACCESS_KEY_ID",
      },
      secretAccessKey: {
        name: "smarthome-mcp-backups",
        key: "AWS_SECRET_ACCESS_KEY",
      },
    });

    const database = resourceByName("smarthome-mcp-database").inputs;
    assert.equal(database.kind, "Database");
    assert.equal((database.spec as any).name, "smarthome_mcp");
    const backup = resourceByName("smarthome-mcp-postgres-backup").inputs;
    assert.equal(backup.kind, "ScheduledBackup");
    assert.equal((backup.spec as any).schedule, "0 30 1 * * *");
    assert.equal((backup.spec as any).method, "barmanObjectStore");
  });

  test("creates one strict confidential Authentik browser application", () => {
    const oauthProviders = resources.filter(
      (candidate) =>
        candidate.type === "authentik:index/providerOauth2:ProviderOauth2",
    );
    assert.equal(oauthProviders.length, 1);
    const provider = oauthProviders[0].inputs;
    assert.equal(provider.clientType, "confidential");
    assert.notEqual(provider.clientSecret, undefined);
    assert.deepEqual(provider.propertyMappings, ["lookup-id"]);
    assert.deepEqual(provider.allowedRedirectUris, [
      {
        matching_mode: "strict",
        url: "https://smarthome-mcp.example.test/oidc/callback",
      },
    ]);
    assert.equal(
      resources.some((candidate) => candidate.name.includes("native")),
      false,
    );
  });

  test("keeps runtime credentials and hosted OAuth settings in Secrets", () => {
    const app = environment();
    assert.match(app.SMARTHOME_MCP_DATABASE_URL as string, /sslmode=verify-full/);
    assert.match(
      app.SMARTHOME_MCP_DATABASE_URL as string,
      /sslrootcert=%2Fvar%2Frun%2Fsecrets%2Fsmarthome-mcp%2Fpostgres%2Fca\.crt/,
    );
    assert.equal(
      app.SMARTHOME_MCP_OIDC_ISSUER,
      "https://auth.example.test/application/o/smarthome-mcp-test-browser/",
    );
    assert.equal(app.SMARTHOME_MCP_OIDC_CLIENT_SECRET, "test-client-secret");
    assert.equal(app.SMARTHOME_MCP_OIDC_SCOPES, "openid");
    assert.equal(app.SMARTHOME_MCP_OAUTH_ISSUER, "https://smarthome-mcp.example.test/oauth");
    assert.equal(app.SMARTHOME_MCP_OAUTH_RESOURCE, "https://smarthome-mcp.example.test/mcp");
    assert.equal(app.SMARTHOME_MCP_OAUTH_ALLOW_DCR, "true");
    assert.equal(app.SMARTHOME_MCP_OAUTH_ALLOW_CIMD, "true");
    assert.equal(
      app.SMARTHOME_MCP_OAUTH_CIMD_TRUSTED_PRIVATE_ORIGINS,
      "https://kuri.internal.example",
    );
    assert.equal(app.SMARTHOME_MCP_OAUTH_ALLOW_LOOPBACK_REDIRECTS, "true");
    assert.equal(
      app.SMARTHOME_MCP_OAUTH_WRAPPING_KEYS_FILE,
      "/var/run/secrets/smarthome-mcp/oauth/keyring.json",
    );
    assert.equal(
      app.SMARTHOME_MCP_HOME_ASSISTANT_URL,
      "https://home-assistant.example.test",
    );
    assert.equal(
      app.SMARTHOME_MCP_HOME_ASSISTANT_TOKEN,
      "test-home-assistant-token",
    );
    assert.equal(app.SMARTHOME_MCP_HOME_ASSISTANT_ALLOW_INSECURE_HTTP, "false");
    assert.equal(app.SMARTHOME_MCP_DEPLOYMENT_ENVIRONMENT, "test");
    assert.equal(app.SMARTHOME_MCP_SERVICE_NAMESPACE, "smarthome");
    assert.equal(
      app.SMARTHOME_MCP_PYROSCOPE_URL,
      "https://telemetry.holdenitdown.net:4040",
    );
    assert.equal(
      app.OTEL_EXPORTER_OTLP_ENDPOINT,
      "https://telemetry.holdenitdown.net:4318",
    );
    assert.equal(app.OTEL_EXPORTER_OTLP_PROTOCOL, "http/protobuf");
    assert.equal(app.OTEL_SERVICE_NAME, "smarthome-mcp");
    assert.equal(
      app.OTEL_RESOURCE_ATTRIBUTES,
      "service.namespace=smarthome,deployment.environment.name=test",
    );

    for (const candidate of resources.filter(
      (entry) =>
        entry.type !== "kubernetes:core/v1:Secret" &&
        entry.type !== "pulumi:index:Stash",
    )) {
      assert.doesNotMatch(
        JSON.stringify(candidate.inputs),
        /SMARTHOME_MCP_(?:DATABASE_URL|OIDC_CLIENT_SECRET|HOME_ASSISTANT_TOKEN)/,
      );
      assert.doesNotMatch(
        JSON.stringify(candidate.inputs),
        /test-home-assistant-token/,
      );
    }
  });

  test("creates a versioned 32-byte keyring with checksum rollout", () => {
    const randomBytes = resource(
      "random:index/randomBytes:RandomBytes",
      "smarthome-mcp-oauth-wrapping-key-v1",
    );
    assert.equal(randomBytes.inputs.length, 32);
    const keyringSecret = resource(
      "kubernetes:core/v1:Secret",
      "smarthome-mcp-oauth-wrapping-keys",
    );
    assert.ok(isSecret(keyringSecret.inputs.stringData));
    const data = unwrapSecrets(keyringSecret.inputs.stringData) as Record<
      string,
      string
    >;
    const keyring = JSON.parse(data["keyring.json"]);
    assert.equal(keyring.schema_version, 1);
    assert.equal(keyring.active, "v1");
    assert.deepEqual(keyring.keys.map((key: any) => key.id), ["v1"]);

    const deployment = resource(
      "kubernetes:apps/v1:Deployment",
      "smarthome-mcp",
    );
    const podTemplate = (unwrapSecrets(deployment.inputs.spec) as any).template;
    assert.notEqual(
      podTemplate.metadata.annotations[
        "smarthome-mcp.holdenitdown.net/wrapping-key-checksum"
      ],
      undefined,
    );
    assert.deepEqual(
      podTemplate.spec.containers[0].volumeMounts.find(
        (mount: any) => mount.name === "oauth-wrapping-keys",
      ),
      {
        name: "oauth-wrapping-keys",
        mountPath: "/var/run/secrets/smarthome-mcp/oauth",
        readOnly: true,
      },
    );
  });

  test("hardens the one-replica Recreate workload and configures probes", () => {
    const deployment = resource(
      "kubernetes:apps/v1:Deployment",
      "smarthome-mcp",
    );
    const spec = unwrapSecrets(deployment.inputs.spec) as any;
    assert.equal(spec.replicas, 1);
    assert.equal(spec.strategy.type, "Recreate");
    assert.equal(
      deployment.inputs.metadata &&
        (deployment.inputs.metadata as any).annotations[
          "secret.reloader.stakater.com/reload"
        ],
      "smarthome-mcp-app,smarthome-mcp-oauth-wrapping-keys",
    );
    const pod = spec.template.spec;
    assert.deepEqual(spec.template.metadata.annotations, {
      "smarthome-mcp.holdenitdown.net/wrapping-key-checksum":
        spec.template.metadata.annotations[
          "smarthome-mcp.holdenitdown.net/wrapping-key-checksum"
        ],
      "resource.opentelemetry.io/service.name": "smarthome-mcp",
      "resource.opentelemetry.io/service.namespace": "smarthome",
      "resource.opentelemetry.io/deployment.environment.name": "test",
    });
    assert.equal(pod.automountServiceAccountToken, false);
    assert.equal(pod.securityContext.runAsUser, 65532);
    assert.equal(pod.securityContext.runAsGroup, 65532);
    assert.equal(pod.securityContext.seccompProfile.type, "RuntimeDefault");
    const container = pod.containers[0];
    assert.match(container.image, /@sha256:[a-f0-9]{64}$/);
    assert.equal(container.ports[0].containerPort, 14334);
    assert.deepEqual(container.envFrom, [
      { secretRef: { name: "smarthome-mcp-app" } },
    ]);
    assert.deepEqual(container.env, [
      {
        name: "SMARTHOME_MCP_K8S_NAMESPACE",
        valueFrom: { fieldRef: { fieldPath: "metadata.namespace" } },
      },
      {
        name: "SMARTHOME_MCP_K8S_POD_NAME",
        valueFrom: { fieldRef: { fieldPath: "metadata.name" } },
      },
      {
        name: "SMARTHOME_MCP_K8S_POD_UID",
        valueFrom: { fieldRef: { fieldPath: "metadata.uid" } },
      },
    ]);
    assert.equal(container.securityContext.allowPrivilegeEscalation, false);
    assert.equal(container.securityContext.readOnlyRootFilesystem, true);
    assert.deepEqual(container.securityContext.capabilities.drop, ["ALL"]);
    assert.equal(container.startupProbe.httpGet.path, "/health");
    assert.equal(container.livenessProbe.httpGet.path, "/health");
    assert.equal(container.readinessProbe.httpGet.path, "/ready");
    assert.deepEqual(container.resources, {
      requests: { cpu: "50m", memory: "64Mi" },
      limits: { cpu: "500m", memory: "256Mi" },
    });
  });

  test("allows patterned DNS, HTTPS, telemetry, Traefik, and PostgreSQL egress", () => {
    const policy = resource(
      "kubernetes:networking.k8s.io/v1:NetworkPolicy",
      "smarthome-mcp-egress",
    );
    const spec = policy.inputs.spec as any;
    assert.deepEqual(spec.policyTypes, ["Egress"]);
    assert.deepEqual(spec.egress, [
      {
        to: [
          {
            namespaceSelector: {
              matchLabels: { "kubernetes.io/metadata.name": "kube-system" },
            },
          },
        ],
        ports: [
          { port: 53, protocol: "UDP" },
          { port: 53, protocol: "TCP" },
        ],
      },
      {
        ports: [
          { port: 443, protocol: "TCP" },
          { port: 4040, protocol: "TCP" },
          { port: 4318, protocol: "TCP" },
        ],
      },
      {
        to: [
          {
            namespaceSelector: {
              matchLabels: { "kubernetes.io/metadata.name": "ingress" },
            },
            podSelector: {
              matchLabels: {
                "app.kubernetes.io/name": "traefik",
                "app.kubernetes.io/instance":
                  "cluster-ingress-ingress-chart-ingress",
              },
            },
          },
        ],
        ports: [{ port: 8443, protocol: "TCP" }],
      },
      {
        to: [
          {
            podSelector: {
              matchLabels: { "cnpg.io/cluster": "smarthome-mcp-postgres" },
            },
          },
        ],
        ports: [{ port: 5432, protocol: "TCP" }],
      },
    ]);
  });

  test("creates the ClusterIP service and streaming Gateway API route", () => {
    const service = resource("kubernetes:core/v1:Service", "smarthome-mcp");
    assert.equal((service.inputs.spec as any).type, "ClusterIP");
    assert.equal((service.inputs.spec as any).ports[0].port, 14334);
    assert.equal((service.inputs.spec as any).ports[0].appProtocol, undefined);
    const route = resourceByName("smarthome-mcp-route").inputs;
    assert.equal(route.kind, "HTTPRoute");
    assert.deepEqual((route.spec as any).parentRefs[0], {
      group: "gateway.networking.k8s.io",
      kind: "Gateway",
      name: "default-gateway",
      namespace: "ingress",
    });
    assert.deepEqual((route.spec as any).hostnames, [
      "smarthome-mcp.example.test",
    ]);
    assert.equal((route.spec as any).rules[0].timeouts.request, "0s");
  });

  test("exports only secret-safe discovery and location values", () => {
    assert.deepEqual(Object.keys(program).sort(), [
      "mcpOAuthIssuer",
      "mcpOAuthJwksUrl",
      "mcpOAuthMetadataUrl",
      "mcpOAuthResource",
      "namespaceNameOutput",
      "publicUrlOutput",
    ]);
    const source = readFileSync(join(__dirname, "index.ts"), "utf8");
    assert.doesNotMatch(source, /export const .*?(?:token|password|secret|key)/i);
  });
});

function resource(type: string, name: string): ResourceRecord {
  const match = [...resources].reverse().find(
    (candidate) => candidate.type === type && candidate.name === name,
  );
  if (!match) throw new Error(`missing mocked resource ${type}::${name}`);
  return match;
}

function resourceByName(name: string): ResourceRecord {
  const match = [...resources]
    .reverse()
    .find((candidate) => candidate.name === name);
  if (!match) throw new Error(`missing mocked resource ${name}`);
  return match;
}

function environment(): Record<string, unknown> {
  const appSecret = resource("kubernetes:core/v1:Secret", "smarthome-mcp-app");
  assert.ok(isSecret(appSecret.inputs.stringData));
  return unwrapSecrets(appSecret.inputs.stringData) as Record<string, unknown>;
}

function isSecret(value: unknown): boolean {
  return Boolean(
    value &&
      typeof value === "object" &&
      Object.keys(value as object).some((key) => key.startsWith("4dabf181")),
  );
}

function unwrapSecrets(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(unwrapSecrets);
  if (!value || typeof value !== "object") return value;
  const record = value as Record<string, unknown>;
  if (isSecret(record) && "value" in record) return unwrapSecrets(record.value);
  return Object.fromEntries(
    Object.entries(record).map(([key, entry]) => [key, unwrapSecrets(entry)]),
  );
}

function stackFile(stack: "preview" | "prod"): string {
  return readFileSync(join(__dirname, `Pulumi.${stack}.yaml`), "utf8");
}

function restoreEnv(name: string, value: string | undefined): void {
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
}
