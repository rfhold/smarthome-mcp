import * as authentik from "@pulumi/authentik";
import * as k8s from "@pulumi/kubernetes";
import * as pulumi from "@pulumi/pulumi";
import * as random from "@pulumi/random";
import * as tls from "@pulumi/tls";
import { createHash } from "node:crypto";
import { OAuthApplication } from "./authentik";
import {
  requireImmutableImage,
  validateHttpsOrigin,
  validateWrappingKeyVersions,
} from "./policy";

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}

const config = new pulumi.Config();
const namespaceName = config.require("namespace");
const hostname = config.require("hostname");
const displayName = config.require("displayName");
const slug = config.require("slug");
const image = requireImmutableImage(config.require("image"));
const authentikBaseUrl = validateHttpsOrigin(
  config.require("authentikBaseUrl"),
  "authentikBaseUrl",
);
const protectData = config.requireBoolean("protectData");
const storageClass = config.require("storageClass");
const databaseStorageSize = config.require("databaseStorageSize");
const backupStorageClass = config.require("backupStorageClass");
const backupEndpoint = validateHttpsOrigin(
  config.require("backupEndpoint"),
  "backupEndpoint",
);
const backupRetention = config.require("backupRetention");
const backupSchedule = config.require("backupSchedule");
const accessTokenTtl = config.require("mcpOAuthAccessTokenTtl");
const refreshTokenTtl = config.require("mcpOAuthRefreshTokenTtl");
const refreshFamilyTtl = config.require("mcpOAuthRefreshFamilyTtl");
const codeTtl = config.require("mcpOAuthCodeTtl");
const mcpOAuthCimdTrustedPrivateOrigins = (
  config.getObject<string[]>("mcpOAuthCimdTrustedPrivateOrigins") ?? []
).map((origin) =>
  validateHttpsOrigin(origin, "mcpOAuthCimdTrustedPrivateOrigins"),
);
const wrappingKeyVersions = config.requireObject<string[]>(
  "mcpOAuthWrappingKeyVersions",
);
const activeWrappingKeyVersion = config.require(
  "mcpOAuthActiveWrappingKeyVersion",
);
validateWrappingKeyVersions(wrappingKeyVersions, activeWrappingKeyVersion);

const homeAssistantUrl = validateHttpsOrigin(
  requireEnv("HOME_ASSISTANT_URL"),
  "HOME_ASSISTANT_URL",
);
const homeAssistantTokenStash = new pulumi.Stash("home-assistant-token", {
  input: pulumi.secret(requireEnv("HOME_ASSISTANT_TOKEN")),
});
const homeAssistantToken = homeAssistantTokenStash.output.apply((value) =>
  String(value),
);

const labels = {
  "app.kubernetes.io/name": "smarthome-mcp",
  "app.kubernetes.io/instance": slug,
  "app.kubernetes.io/part-of": "smarthome-mcp",
  "app.kubernetes.io/managed-by": "pulumi",
};
const workloadLabels = { ...labels, "app.kubernetes.io/component": "server" };
const deploymentEnvironment = pulumi.getStack();
const publicUrl = `https://${hostname}`;
const browserCallback = `${publicUrl}/oidc/callback`;
const mcpIssuer = `${publicUrl}/oauth`;
const mcpResource = `${publicUrl}/mcp`;
const wrappingKeyMountPath = "/var/run/secrets/smarthome-mcp/oauth";
const wrappingKeyFile = `${wrappingKeyMountPath}/keyring.json`;
const postgresTrustMountPath = "/var/run/secrets/smarthome-mcp/postgres";
const postgresCaFile = `${postgresTrustMountPath}/ca.crt`;

const namespace = new k8s.core.v1.Namespace("smarthome-mcp-namespace", {
  metadata: { name: namespaceName, labels },
});

const backupBucket = new k8s.apiextensions.CustomResource(
  "smarthome-mcp-backups-bucket",
  {
    apiVersion: "objectbucket.io/v1alpha1",
    kind: "ObjectBucketClaim",
    metadata: {
      name: "smarthome-mcp-backups",
      namespace: namespace.metadata.name,
      labels,
    },
    spec: {
      storageClassName: backupStorageClass,
      generateBucketName: `${slug}-backups`,
    },
  },
  { dependsOn: [namespace], protect: protectData },
);
const backupConfig = pulumi
  .all([namespace.metadata.name, backupBucket.id])
  .apply(([resolvedNamespace]) =>
    k8s.core.v1.ConfigMap.get(
      "smarthome-mcp-backups-generated-config",
      `${resolvedNamespace}/smarthome-mcp-backups`,
    ),
  );

const databaseCluster = new k8s.apiextensions.CustomResource(
  "smarthome-mcp-postgres",
  {
    apiVersion: "postgresql.cnpg.io/v1",
    kind: "Cluster",
    metadata: {
      name: "smarthome-mcp-postgres",
      namespace: namespace.metadata.name,
      labels,
    },
    spec: {
      instances: 1,
      enableSuperuserAccess: false,
      backup: {
        retentionPolicy: backupRetention,
        barmanObjectStore: {
          destinationPath: backupConfig.data.apply(
            (values) => `s3://${values.BUCKET_NAME}/database`,
          ),
          endpointURL: backupEndpoint,
          s3Credentials: {
            accessKeyId: {
              name: "smarthome-mcp-backups",
              key: "AWS_ACCESS_KEY_ID",
            },
            secretAccessKey: {
              name: "smarthome-mcp-backups",
              key: "AWS_SECRET_ACCESS_KEY",
            },
          },
          wal: { compression: "gzip" },
          data: { compression: "gzip" },
        },
      },
      storage: { size: databaseStorageSize, storageClass },
      resources: {
        requests: { cpu: "100m", memory: "256Mi" },
        limits: { cpu: "1", memory: "1Gi" },
      },
    },
  },
  { dependsOn: [backupBucket], protect: protectData },
);

new k8s.apiextensions.CustomResource(
  "smarthome-mcp-postgres-backup",
  {
    apiVersion: "postgresql.cnpg.io/v1",
    kind: "ScheduledBackup",
    metadata: {
      name: "smarthome-mcp-postgres",
      namespace: namespace.metadata.name,
      labels,
    },
    spec: {
      schedule: backupSchedule,
      backupOwnerReference: "self",
      cluster: { name: "smarthome-mcp-postgres" },
      immediate: true,
      method: "barmanObjectStore",
    },
  },
  { dependsOn: [databaseCluster] },
);

const database = new k8s.apiextensions.CustomResource(
  "smarthome-mcp-database",
  {
    apiVersion: "postgresql.cnpg.io/v1",
    kind: "Database",
    metadata: {
      name: "smarthome-mcp",
      namespace: namespace.metadata.name,
      labels,
    },
    spec: {
      name: "smarthome_mcp",
      owner: "app",
      cluster: { name: "smarthome-mcp-postgres" },
    },
  },
  { dependsOn: [databaseCluster] },
);

const signingPrivateKey = new tls.PrivateKey("smarthome-mcp-oidc-signing-key", {
  algorithm: "RSA",
  rsaBits: 4096,
});
const signingCertificate = new tls.SelfSignedCert(
  "smarthome-mcp-oidc-signing-certificate",
  {
    privateKeyPem: signingPrivateKey.privateKeyPem,
    subject: { commonName: `${slug} OAuth signing` },
    validityPeriodHours: 87600,
    allowedUses: ["digital_signature", "key_encipherment"],
  },
);
const signingKey = new authentik.CertificateKeyPair(
  "smarthome-mcp-oidc-signing-keypair",
  {
    name: `${displayName} OAuth signing key`,
    certificateData: signingCertificate.certPem,
    keyData: signingPrivateKey.privateKeyPem,
  },
);
const openid = authentik.getPropertyMappingProviderScopeOutput({
  managed: "goauthentik.io/providers/oauth2/scope-openid",
});
const browserApp = new OAuthApplication("smarthome-mcp-browser", {
  displayName: `${displayName} Browser`,
  slug: `${slug}-browser`,
  issuerBaseUrl: authentikBaseUrl,
  redirectUri: browserCallback,
  signingKeyId: signingKey.id,
  propertyMappings: [openid.id],
  launchUrl: publicUrl,
});

const cnpgAppSecret = pulumi
  .all([namespace.metadata.name, databaseCluster.id])
  .apply(([resolvedNamespace]) =>
    k8s.core.v1.Secret.get(
      "smarthome-mcp-postgres-generated-app",
      `${resolvedNamespace}/smarthome-mcp-postgres-app`,
    ),
  );
const decodeDatabaseSecret = (key: string) =>
  cnpgAppSecret.data.apply((values) =>
    Buffer.from(values[key], "base64").toString("utf8"),
  );
const databaseUrl = pulumi
  .all([decodeDatabaseSecret("username"), decodeDatabaseSecret("password")])
  .apply(
    ([username, password]) =>
      `postgresql://${encodeURIComponent(username)}:${encodeURIComponent(password)}@smarthome-mcp-postgres-rw.${namespaceName}.svc:5432/smarthome_mcp?sslmode=verify-full&sslrootcert=${encodeURIComponent(postgresCaFile)}`,
  );

const wrappingKeys = wrappingKeyVersions.map((version) => ({
  version,
  key: new random.RandomBytes(
    `smarthome-mcp-oauth-wrapping-key-${version}`,
    { length: 32 },
  ).base64,
}));
const wrappingKeyring = pulumi.secret(
  pulumi
    .all(wrappingKeys.map(({ key }) => key))
    .apply((keys) =>
      JSON.stringify({
        schema_version: 1,
        active: activeWrappingKeyVersion,
        keys: wrappingKeyVersions.map((version, index) => ({
          id: version,
          key: Buffer.from(keys[index], "base64").toString("base64url"),
        })),
      }),
    ),
);
const wrappingKeyChecksum = wrappingKeyring.apply((keyring) =>
  createHash("sha256").update(keyring).digest("hex"),
);
const wrappingKeySecret = new k8s.core.v1.Secret(
  "smarthome-mcp-oauth-wrapping-keys",
  {
    metadata: {
      name: "smarthome-mcp-oauth-wrapping-keys",
      namespace: namespace.metadata.name,
      labels,
      annotations: { "smarthome-mcp.holdenitdown.net/keyring-format": "1" },
    },
    type: "Opaque",
    stringData: { "keyring.json": wrappingKeyring },
  },
);

const appSecret = new k8s.core.v1.Secret(
  "smarthome-mcp-app",
  {
    metadata: {
      name: "smarthome-mcp-app",
      namespace: namespace.metadata.name,
      labels,
    },
    stringData: {
      SMARTHOME_MCP_DATABASE_URL: databaseUrl,
      SMARTHOME_MCP_PUBLIC_URL: publicUrl,
      SMARTHOME_MCP_OIDC_ISSUER: browserApp.issuer,
      SMARTHOME_MCP_OIDC_CLIENT_ID: browserApp.clientId,
      SMARTHOME_MCP_OIDC_CLIENT_SECRET: browserApp.clientSecret,
      SMARTHOME_MCP_OIDC_REDIRECT_URI: browserCallback,
      SMARTHOME_MCP_OIDC_SCOPES: "openid",
      SMARTHOME_MCP_OAUTH_ISSUER: mcpIssuer,
      SMARTHOME_MCP_OAUTH_RESOURCE: mcpResource,
      SMARTHOME_MCP_OAUTH_REQUIRED_SCOPE: "mcp:read",
      SMARTHOME_MCP_OAUTH_ACCESS_TOKEN_TTL: accessTokenTtl,
      SMARTHOME_MCP_OAUTH_REFRESH_TOKEN_TTL: refreshTokenTtl,
      SMARTHOME_MCP_OAUTH_REFRESH_FAMILY_TTL: refreshFamilyTtl,
      SMARTHOME_MCP_OAUTH_CODE_TTL: codeTtl,
      SMARTHOME_MCP_OAUTH_ALLOW_DCR: "true",
      SMARTHOME_MCP_OAUTH_ALLOW_CIMD: "true",
      ...(mcpOAuthCimdTrustedPrivateOrigins.length > 0
        ? {
            SMARTHOME_MCP_OAUTH_CIMD_TRUSTED_PRIVATE_ORIGINS:
              mcpOAuthCimdTrustedPrivateOrigins.join(","),
          }
        : {}),
      SMARTHOME_MCP_OAUTH_ALLOW_LOOPBACK_REDIRECTS: "true",
      SMARTHOME_MCP_OAUTH_WRAPPING_KEYS_FILE: wrappingKeyFile,
      SMARTHOME_MCP_HOME_ASSISTANT_URL: homeAssistantUrl,
      SMARTHOME_MCP_HOME_ASSISTANT_TOKEN: homeAssistantToken,
      SMARTHOME_MCP_HOME_ASSISTANT_ALLOW_INSECURE_HTTP: "false",
      SMARTHOME_MCP_DEPLOYMENT_ENVIRONMENT: deploymentEnvironment,
      SMARTHOME_MCP_SERVICE_NAMESPACE: "smarthome",
      SMARTHOME_MCP_PYROSCOPE_URL: "https://telemetry.holdenitdown.net:4040",
      OTEL_EXPORTER_OTLP_ENDPOINT: "https://telemetry.holdenitdown.net:4318",
      OTEL_EXPORTER_OTLP_PROTOCOL: "http/protobuf",
      OTEL_SERVICE_NAME: "smarthome-mcp",
      OTEL_RESOURCE_ATTRIBUTES: `service.namespace=smarthome,deployment.environment.name=${deploymentEnvironment}`,
    },
  },
  { dependsOn: [database, browserApp] },
);

new k8s.apps.v1.Deployment(
  "smarthome-mcp",
  {
    metadata: {
      name: "smarthome-mcp",
      namespace: namespace.metadata.name,
      labels: workloadLabels,
      annotations: {
        "secret.reloader.stakater.com/reload":
          "smarthome-mcp-app,smarthome-mcp-oauth-wrapping-keys",
      },
    },
    spec: {
      replicas: 1,
      strategy: { type: "Recreate" },
      selector: { matchLabels: workloadLabels },
      template: {
        metadata: {
          labels: workloadLabels,
          annotations: {
            "smarthome-mcp.holdenitdown.net/wrapping-key-checksum":
              wrappingKeyChecksum,
            "resource.opentelemetry.io/service.name": "smarthome-mcp",
            "resource.opentelemetry.io/service.namespace": "smarthome",
            "resource.opentelemetry.io/deployment.environment.name":
              deploymentEnvironment,
          },
        },
        spec: {
          automountServiceAccountToken: false,
          securityContext: {
            runAsNonRoot: true,
            runAsUser: 65532,
            runAsGroup: 65532,
            fsGroup: 65532,
            fsGroupChangePolicy: "OnRootMismatch",
            seccompProfile: { type: "RuntimeDefault" },
          },
          terminationGracePeriodSeconds: 30,
          containers: [
            {
              name: "smarthome-mcp",
              image,
              imagePullPolicy: "IfNotPresent",
              ports: [
                { name: "http", containerPort: 14334, protocol: "TCP" },
              ],
              envFrom: [{ secretRef: { name: appSecret.metadata.name } }],
              env: [
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
              ],
              securityContext: {
                runAsNonRoot: true,
                allowPrivilegeEscalation: false,
                readOnlyRootFilesystem: true,
                capabilities: { drop: ["ALL"] },
              },
              resources: {
                requests: { cpu: "50m", memory: "64Mi" },
                limits: { cpu: "500m", memory: "256Mi" },
              },
              startupProbe: {
                httpGet: { path: "/health", port: "http" },
                periodSeconds: 2,
                failureThreshold: 60,
              },
              readinessProbe: {
                httpGet: { path: "/ready", port: "http" },
                periodSeconds: 5,
                failureThreshold: 3,
              },
              livenessProbe: {
                httpGet: { path: "/health", port: "http" },
                periodSeconds: 10,
                failureThreshold: 3,
              },
              volumeMounts: [
                { name: "tmp", mountPath: "/tmp" },
                {
                  name: "oauth-wrapping-keys",
                  mountPath: wrappingKeyMountPath,
                  readOnly: true,
                },
                {
                  name: "postgres-ca",
                  mountPath: postgresTrustMountPath,
                  readOnly: true,
                },
              ],
            },
          ],
          volumes: [
            { name: "tmp", emptyDir: { sizeLimit: "64Mi" } },
            {
              name: "oauth-wrapping-keys",
              secret: {
                secretName: wrappingKeySecret.metadata.name,
                defaultMode: 0o440,
                items: [
                  { key: "keyring.json", path: "keyring.json", mode: 0o440 },
                ],
              },
            },
            {
              name: "postgres-ca",
              secret: {
                secretName: "smarthome-mcp-postgres-ca",
                defaultMode: 0o444,
                items: [{ key: "ca.crt", path: "ca.crt", mode: 0o444 }],
              },
            },
          ],
        },
      },
    },
  },
  { dependsOn: [appSecret, wrappingKeySecret] },
);

new k8s.networking.v1.NetworkPolicy("smarthome-mcp-egress", {
  metadata: {
    name: "smarthome-mcp-egress",
    namespace: namespace.metadata.name,
    labels,
  },
  spec: {
    podSelector: { matchLabels: workloadLabels },
    policyTypes: ["Egress"],
    egress: [
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
    ],
  },
});

const service = new k8s.core.v1.Service("smarthome-mcp", {
  metadata: {
    name: "smarthome-mcp",
    namespace: namespace.metadata.name,
    labels,
  },
  spec: {
    type: "ClusterIP",
    selector: workloadLabels,
    ports: [
      {
        name: "http",
        port: 14334,
        targetPort: "http",
      },
    ],
  },
});

new k8s.apiextensions.CustomResource("smarthome-mcp-route", {
  apiVersion: "gateway.networking.k8s.io/v1",
  kind: "HTTPRoute",
  metadata: {
    name: "smarthome-mcp",
    namespace: namespace.metadata.name,
    labels,
  },
  spec: {
    parentRefs: [
      {
        group: "gateway.networking.k8s.io",
        kind: "Gateway",
        name: "default-gateway",
        namespace: "ingress",
      },
    ],
    hostnames: [hostname],
    rules: [
      {
        matches: [{ path: { type: "PathPrefix", value: "/" } }],
        backendRefs: [{ name: service.metadata.name, port: 14334 }],
        timeouts: { request: "0s" },
      },
    ],
  },
});

export const namespaceNameOutput = namespace.metadata.name;
export const publicUrlOutput = publicUrl;
export const mcpOAuthIssuer = mcpIssuer;
export const mcpOAuthResource = mcpResource;
export const mcpOAuthMetadataUrl = `${publicUrl}/.well-known/oauth-authorization-server/oauth`;
export const mcpOAuthJwksUrl = `${mcpIssuer}/jwks.json`;
