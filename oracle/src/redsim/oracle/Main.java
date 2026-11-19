package redsim.oracle;

import java.io.BufferedReader;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.PrintStream;
import java.io.Writer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.concurrent.LinkedBlockingQueue;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.util.Util;
import net.minecraft.world.level.storage.LevelStorageSource;

/**
 * Entry point of the redsim oracle: a headless Minecraft server running the real
 * game code, driven over a JSON-lines protocol on stdin/stdout.
 *
 * Protocol stdout is reserved for responses; all game logging goes to stderr.
 */
public final class Main {
    public static Writer protocolOut;
    public static final LinkedBlockingQueue<String> requests = new LinkedBlockingQueue<>();
    public static volatile boolean stdinClosed = false;
    /** false for world export: no force-loaded test slots */
    public static boolean slots = true;

    public static void main(String[] args) throws Exception {
        // Keep the real stdout for the protocol, send everything else to stderr.
        protocolOut = new OutputStreamWriter(new FileOutputStream(FileDescriptor.out), StandardCharsets.UTF_8);
        System.setOut(new PrintStream(new FileOutputStream(FileDescriptor.err), true, StandardCharsets.UTF_8));

        String universe = args.length > 0 ? args[0] : "oracle/run";
        slots = !(args.length > 1 && args[1].equals("noslots"));
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Util.startTimerHackThread();

        Path universeDir = Paths.get(universe);
        deleteRecursively(universeDir);
        Files.createDirectories(universeDir);
        LevelStorageSource.LevelStorageAccess storage = LevelStorageSource.createDefault(universeDir).createAccess("oracleworld");
        PackRepository packs = ServerPacksSource.createPackRepository(storage);
        OracleServer server = MinecraftServer.spin(thread -> OracleServer.create(thread, storage, packs));

        Thread reader = new Thread(() -> {
            try (BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8))) {
                String line;
                while ((line = in.readLine()) != null) {
                    if (!line.isBlank()) {
                        requests.put(line);
                    }
                }
            } catch (Exception e) {
                e.printStackTrace();
            }
            stdinClosed = true;
            requests.offer("{\"op\":\"quit\"}");
        }, "oracle-stdin");
        reader.setDaemon(true);
        reader.start();
    }

    public static synchronized void respond(String json) {
        try {
            protocolOut.write(json);
            protocolOut.write('\n');
            protocolOut.flush();
        } catch (Exception e) {
            e.printStackTrace();
        }
    }

    private static void deleteRecursively(Path p) throws java.io.IOException {
        if (!Files.exists(p)) return;
        try (var walk = Files.walk(p)) {
            walk.sorted(java.util.Comparator.reverseOrder()).forEach(q -> q.toFile().delete());
        }
    }
}
