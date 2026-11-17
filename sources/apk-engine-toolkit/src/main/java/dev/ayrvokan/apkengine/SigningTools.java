package dev.ayrvokan.apkengine;

import com.google.gson.*;
import com.android.apksig.ApkSigner;
import com.android.apksig.ApkVerifier;
import java.io.*;
import java.nio.file.*;
import java.security.*;
import java.security.cert.X509Certificate;
import java.util.*;
import static dev.ayrvokan.apkengine.Main.*;

final class SigningTools {
    static void register() {
        tool("verify_apk", "apksig", "Verify APK signatures and return signer certificate digests.", false, new String[]{"path"},
                obj("path", field("string", "APK path"), "min_sdk", field("integer", "Optional minimum Android API; otherwise read the manifest"), "max_sdk", field("integer", "Optional maximum Android API")), SigningTools::verify);
        tool("sign_apk", "apksig", "Sign an APK with a supplied PKCS12/JKS key; password comes from an environment variable.", true, new String[]{"path", "output", "keystore", "alias"},
                obj("path", field("string", "Input APK"), "output", field("string", "Output signed APK"), "keystore", field("string", "PKCS12 or JKS keystore"), "alias", field("string", "Key alias"),
                    "keystore_type", field("string", "PKCS12 by default"), "password_env", field("string", "Environment name, default APK_ENGINE_KEYSTORE_PASSWORD"), "key_password_env", field("string", "Optional different private-key password environment"),
                    "min_sdk", field("integer", "Optional minimum Android API; otherwise read the manifest"), "v1", field("boolean", "Enable v1, default true"), "v2", field("boolean", "Enable v2, default true"), "v3", field("boolean", "Enable v3, default true"), "overwrite", field("boolean", "Replace existing output, default false")), SigningTools::sign);
    }

    static JsonObject verify(JsonObject args) throws Exception {
        Path path = input(args, "path");
        ApkVerifier.Builder builder = new ApkVerifier.Builder(path.toFile());
        if (args.has("min_sdk")) builder.setMinCheckedPlatformVersion(integer(args, "min_sdk", 24, 1, 37));
        if (args.has("max_sdk")) builder.setMaxCheckedPlatformVersion(integer(args, "max_sdk", 37, 1, 37));
        ApkVerifier.Result result = builder.build().verify();
        JsonArray certificates = new JsonArray(), errors = new JsonArray(), warnings = new JsonArray();
        for (X509Certificate cert : result.getSignerCertificates()) certificates.add(obj("subject", cert.getSubjectX500Principal().getName(), "sha256", HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(cert.getEncoded()))));
        for (var issue : result.getAllErrors()) errors.add(issue.toString());
        for (var issue : result.getWarnings()) warnings.add(issue.toString());
        return obj("artifact", artifact(path), "verified", result.isVerified(), "v1", result.isVerifiedUsingV1Scheme(), "v2", result.isVerifiedUsingV2Scheme(), "v3", result.isVerifiedUsingV3Scheme(), "certificates", certificates, "errors", errors, "warnings", warnings);
    }

    static char[] password(JsonObject args, String key, String fallback) {
        String name = args.has(key) ? string(args, key) : fallback;
        String password = System.getenv(name);
        if (password == null) throw new IllegalArgumentException("Required environment variable is not set: " + name);
        return password.toCharArray();
    }

    static JsonObject sign(JsonObject args) throws Exception {
        Path source = input(args, "path"), target = output(args, "output"), store = input(args, "keystore");
        different(source, target);
        different(store, target);
        String passwordName = args.has("password_env") ? string(args, "password_env") : "APK_ENGINE_KEYSTORE_PASSWORD";
        char[] password = password(args, "password_env", passwordName), keyPassword = password(args, "key_password_env", passwordName);
        try {
            KeyStore keys = KeyStore.getInstance(args.has("keystore_type") ? string(args, "keystore_type") : "PKCS12");
            try (InputStream in = Files.newInputStream(store)) { keys.load(in, password); }
            String alias = string(args, "alias");
            Key key = keys.getKey(alias, keyPassword);
            if (!(key instanceof PrivateKey privateKey)) throw new IllegalArgumentException("Alias is not a private key: " + alias);
            var chain = keys.getCertificateChain(alias);
            if (chain == null || chain.length == 0) throw new IllegalArgumentException("No certificate chain for alias: " + alias);
            List<X509Certificate> certificates = new ArrayList<>();
            for (var certificate : chain) certificates.add((X509Certificate)certificate);
            var signer = new ApkSigner.SignerConfig.Builder(alias, privateKey, certificates).build();
            JsonObject output = writeArtifact(args, target, temporary -> {
                var builder = new ApkSigner.Builder(List.of(signer)).setInputApk(source.toFile()).setOutputApk(temporary.toFile())
                        .setV1SigningEnabled(bool(args, "v1", true)).setV2SigningEnabled(bool(args, "v2", true)).setV3SigningEnabled(bool(args, "v3", true)).setV4SigningEnabled(false);
                if (args.has("min_sdk")) builder.setMinSdkVersion(integer(args, "min_sdk", 24, 1, 37));
                builder.build().sign();
            });
            JsonObject check = obj("path", target.toString());
            if (args.has("min_sdk")) check.add("min_sdk", args.get("min_sdk"));
            output.add("verification", verify(check));
            return output;
        } finally { Arrays.fill(password, '\0'); Arrays.fill(keyPassword, '\0'); }
    }
}
