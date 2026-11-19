package redsim.oracle;

import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mojang.authlib.GameProfile;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.TimeUnit;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.Registries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.Container;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.ButtonBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.ComparatorBlockEntity;
import net.minecraft.world.level.block.piston.PistonMovingBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;

/**
 * Runs scenarios against the real game. Everything here executes on the server
 * thread at the end of a server tick ("between ticks", where player actions land).
 *
 * Timeline of a scenario: step 0 = right after placement; step t = after game tick t.
 * Actions with at=t run after step t is recorded (so they affect tick t+1).
 */
public final class Harness {
    static final int SLOT_SPACING = 96;
    static final int GRID = 6;
    static final int BASE_X = 4096, BASE_Z = 4096, BASE_Y = 0;
    static final int MAX_H = 64, MAX_XZ = 64;

    private final OracleServer server;
    private ServerLevel level;
    private final Gson gson = new Gson();
    private final ArrayDeque<Scenario> waiting = new ArrayDeque<>();
    private final Scenario[] slots = new Scenario[GRID * GRID];
    private Player player;
    private final List<Long> forcedChunks = new ArrayList<>();
    private boolean ready = false;

    Harness(OracleServer server) {
        this.server = server;
    }

    void init() {
        this.level = server.overworld();
        for (int i = 0; i < (Main.slots ? GRID : 0); i++) {
            for (int j = 0; j < GRID; j++) {
                int x0 = BASE_X + i * SLOT_SPACING, z0 = BASE_Z + j * SLOT_SPACING;
                for (int cx = (x0 >> 4) - 1; cx <= ((x0 + MAX_XZ) >> 4) + 1; cx++) {
                    for (int cz = (z0 >> 4) - 1; cz <= ((z0 + MAX_XZ) >> 4) + 1; cz++) {
                        level.setChunkForced(cx, cz, true);
                        forcedChunks.add(net.minecraft.world.level.ChunkPos.pack(cx, cz));
                    }
                }
            }
        }
        this.player = new Player(level, new GameProfile(UUID.fromString("00000000-0000-0000-0000-00000000aaaa"), "redsim")) {
            @Override public GameType gameMode() { return GameType.CREATIVE; }
            @Override public boolean isClientAuthoritative() { return false; }
        };
        GameType.CREATIVE.updatePlayerAbilities(player.getAbilities());
    }

    boolean busy() {
        if (!waiting.isEmpty()) return true;
        for (Scenario s : slots) if (s != null) return true;
        return false;
    }

    /** Called between ticks when the server would otherwise sleep. */
    void idleWait() {
        if (!ready || busy() || !Main.requests.isEmpty()) return;
        try {
            String line = Main.requests.poll(20, TimeUnit.MILLISECONDS);
            if (line != null) handle(line);
        } catch (InterruptedException ignored) {
        }
    }

    /** Slots are usable once every forced chunk is block-ticking with entities loaded. */
    private boolean checkReady() {
        for (long c : forcedChunks) {
            if (!level.shouldTickBlocksAt(c) || !level.isPositionTickingWithEntitiesLoaded(c)) return false;
        }
        return true;
    }

    /**
     * The game runs light updates on a background thread, so a light change reaches readers
     * (daylight detectors) "soon". Make that deterministic: before each tick, wait until every
     * queued light update for scenarios that read light has been applied.
     */
    void settleLight() {
        var le = level.getChunkSource().getLightEngine();
        List<java.util.concurrent.CompletableFuture<?>> fs = new ArrayList<>();
        for (Scenario sc : slots) {
            if (sc == null || !sc.needsLight) continue;
            for (int cx = (sc.ox - 16) >> 4; cx <= (sc.ox + sc.sx + 16) >> 4; cx++) {
                for (int cz = (sc.oz - 16) >> 4; cz <= (sc.oz + sc.sz + 16) >> 4; cz++) {
                    fs.add(le.waitForPendingTasks(cx, cz));
                }
            }
        }
        if (fs.isEmpty()) return;
        long end = System.nanoTime() + 10_000_000_000L;
        while (System.nanoTime() < end) {
            boolean done = true;
            for (var f : fs) if (!f.isDone()) { done = false; break; }
            if (done) return;
            le.tryScheduleUpdate();
            Thread.onSpinWait();
        }
        System.err.println("redsim oracle: light updates did not settle");
    }

    void afterTick() {
        if (!ready) {
            if (!checkReady()) return;
            ready = true;
            System.err.println("[redsim-oracle] ready at game time " + level.getGameTime());
            Main.respond("{\"op\":\"ready\",\"version\":\"" + net.minecraft.SharedConstants.getCurrentVersion().name() + "\"}");
        }
        String line;
        while ((line = Main.requests.poll()) != null) handle(line);

        for (int i = 0; i < slots.length; i++) {
            Scenario s = slots[i];
            if (s == null) continue;
            try {
                s.step++;
                s.recordTick();
                if (s.step >= s.ticks) {
                    s.finish();
                    slots[i] = null;
                } else {
                    s.runActions();
                }
            } catch (Throwable t) {
                t.printStackTrace();
                Main.respond(error(s.id, t));
                slots[i] = null;
            }
        }
        for (int i = 0; i < slots.length && !waiting.isEmpty(); i++) {
            if (slots[i] == null) {
                Scenario head = waiting.peek();
                if (!head.chunksReady()) {
                    // absolute-origin scenario: wait (in order) until its chunks are fully ticking
                    break;
                }
                Scenario s = waiting.poll();
                try {
                    s.start(i);
                    if (s.ticks <= 0) {
                        s.finish();
                    } else {
                        slots[i] = s;
                    }
                } catch (Throwable t) {
                    t.printStackTrace();
                    Main.respond(error(s.id, t));
                }
            }
        }
    }

    private String error(String id, Throwable t) {
        JsonObject o = new JsonObject();
        o.addProperty("id", id);
        o.addProperty("ok", false);
        o.addProperty("error", t.toString());
        return gson.toJson(o);
    }

    void handle(String line) {
        JsonObject req = JsonParser.parseString(line).getAsJsonObject();
        String op = req.get("op").getAsString();
        String id = req.has("id") ? req.get("id").getAsString() : "";
        try {
            switch (op) {
                case "quit" -> server.halt(false);
                case "ping" -> Main.respond("{\"op\":\"pong\",\"id\":\"" + id + "\",\"time\":" + level.getGameTime() + "}");
                case "extract" -> {
                    Extractor.extract(server, java.nio.file.Paths.get(req.get("out").getAsString()));
                    Main.respond("{\"id\":\"" + id + "\",\"ok\":true}");
                }
                case "run" -> waiting.add(new Scenario(req));
                case "set_day_time" -> {
                    // the overworld clock is shared by every slot: only change it between batches
                    var clock = level.dimensionType().defaultClock().orElseThrow();
                    server.clockManager().setTotalTicks(clock, req.get("time").getAsLong());
                    level.environmentAttributes().invalidateTickCache();
                    level.updateSkyBrightness();
                    Main.respond("{\"id\":\"" + id + "\",\"ok\":true}");
                }
                case "load_structure" -> {
                    var tag = net.minecraft.nbt.NbtIo.readCompressed(java.nio.file.Paths.get(req.get("path").getAsString()), net.minecraft.nbt.NbtAccounter.unlimitedHeap());
                    var template = new net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplate();
                    template.load(level.holderLookup(Registries.BLOCK), tag);
                    var size = template.getSize();
                    BlockPos o = new BlockPos(BASE_X + SLOT_SPACING * GRID + 200, 0, BASE_Z);
                    level.getChunk(o);
                    template.placeInWorld(level, o, o, new net.minecraft.world.level.levelgen.structure.templatesystem.StructurePlaceSettings(), level.getRandom(), 2 | 16);
                    JsonArray out = new JsonArray();
                    for (int y = 0; y < size.getY(); y++) for (int z = 0; z < size.getZ(); z++) for (int x = 0; x < size.getX(); x++) {
                        BlockState st = level.getBlockState(o.offset(x, y, z));
                        if (!st.isAir()) {
                            JsonArray c = new JsonArray();
                            c.add(x); c.add(y); c.add(z); c.add(BlockStateParser.serialize(st));
                            out.add(c);
                        }
                    }
                    JsonObject r = new JsonObject();
                    r.addProperty("id", id); r.addProperty("ok", true); r.add("blocks", out);
                    Main.respond(gson.toJson(r));
                }
                case "save_world" -> {
                    saveWorld(req);
                    Main.respond("{\"id\":\"" + id + "\",\"ok\":true}");
                }
                default -> throw new IllegalArgumentException("unknown op " + op);
            }
        } catch (Throwable t) {
            t.printStackTrace();
            Main.respond(error(id, t));
        }
    }

    /** Place a build at an origin in this (fresh) world, save, and copy the world folder out. */
    void saveWorld(JsonObject req) throws Exception {
        JsonArray o = req.getAsJsonArray("origin");
        int ox = o.get(0).getAsInt(), oy = o.get(1).getAsInt(), oz = o.get(2).getAsInt();
        JsonArray size = req.getAsJsonArray("size");
        for (int cx = (ox >> 4) - 1; cx <= ((ox + size.get(0).getAsInt()) >> 4) + 1; cx++) {
            for (int cz = (oz >> 4) - 1; cz <= ((oz + size.get(2).getAsInt()) >> 4) + 1; cz++) {
                level.getChunk(cx, cz);
            }
        }
        for (JsonElement el : req.getAsJsonArray("blocks")) {
            JsonArray b = el.getAsJsonArray();
            BlockPos p = new BlockPos(ox + b.get(0).getAsInt(), oy + b.get(1).getAsInt(), oz + b.get(2).getAsInt());
            level.setBlock(p, parseState(b.get(3)), 2 | 16 | 32 | 256 | 512);
        }
        if (req.has("block_entities")) {
            for (JsonElement el : req.getAsJsonArray("block_entities")) {
                JsonObject e = el.getAsJsonObject();
                JsonArray pos = e.getAsJsonArray("pos");
                BlockEntity be = level.getBlockEntity(new BlockPos(ox + pos.get(0).getAsInt(), oy + pos.get(1).getAsInt(), oz + pos.get(2).getAsInt()));
                if (be instanceof ComparatorBlockEntity c && e.has("output")) c.setOutputSignal(e.get("output").getAsInt());
                if (be instanceof Container && e.has("items")) {
                    java.util.List<ItemStack> list = containerItems(be);
                    for (JsonElement it : e.getAsJsonArray("items")) {
                        JsonArray a = it.getAsJsonArray();
                        var item = net.minecraft.core.registries.BuiltInRegistries.ITEM.getValue(net.minecraft.resources.Identifier.parse(a.get(1).getAsString()));
                        list.set(a.get(0).getAsInt(), new ItemStack(item, a.get(2).getAsInt()));
                    }
                    be.setChanged();
                }
            }
        }
        BlockPos spawn = new BlockPos(ox - 3, oy, oz - 3);
        level.setRespawnData(net.minecraft.world.level.storage.LevelData.RespawnData.of(level.dimension(), spawn, -45.0F, 20.0F));
        server.saveEverything(true, true, true);
        java.nio.file.Path src = server.storageSource().getLevelPath(net.minecraft.world.level.storage.LevelResource.ROOT).toAbsolutePath().normalize();
        java.nio.file.Path dst = java.nio.file.Paths.get(req.get("out").getAsString());
        try (var walk = java.nio.file.Files.walk(src)) {
            for (java.nio.file.Path p : (Iterable<java.nio.file.Path>) walk::iterator) {
                java.nio.file.Path rel = src.relativize(p);
                if (rel.toString().equals("session.lock")) continue;
                java.nio.file.Path t = dst.resolve(rel.toString());
                if (java.nio.file.Files.isDirectory(p)) java.nio.file.Files.createDirectories(t);
                else java.nio.file.Files.copy(p, t, java.nio.file.StandardCopyOption.REPLACE_EXISTING);
            }
        }
    }

    /** RedstoneTorchBlock.RECENT_TOGGLES is level-global: drop entries left by earlier scenarios. */
    @SuppressWarnings("unchecked")
    void clearTorchToggles(BoundingBox box) {
        try {
            var f = net.minecraft.world.level.block.RedstoneTorchBlock.class.getDeclaredField("RECENT_TOGGLES");
            f.setAccessible(true);
            var map = (Map<Object, List<Object>>) f.get(null);
            List<Object> list = map.get(level);
            if (list == null) return;
            var pf = net.minecraft.world.level.block.RedstoneTorchBlock.Toggle.class.getDeclaredField("pos");
            pf.setAccessible(true);
            list.removeIf(t -> {
                try { return box.isInside((BlockPos) pf.get(t)); } catch (Exception e) { return false; }
            });
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }

    /** True if the scenario places a daylight detector (states may be ids or strings). */
    boolean readsLight(JsonObject req) {
        try {
            if (req.has("blocks")) {
                for (JsonElement b : req.getAsJsonArray("blocks")) {
                    if (parseState(b.getAsJsonArray().get(3)).getBlock() instanceof net.minecraft.world.level.block.DaylightDetectorBlock) return true;
                }
            }
            if (req.has("actions")) {
                for (JsonElement a : req.getAsJsonArray("actions")) {
                    JsonObject ao = a.getAsJsonObject();
                    if (ao.has("state") && parseState(ao.get("state")).getBlock() instanceof net.minecraft.world.level.block.DaylightDetectorBlock) return true;
                }
            }
        } catch (Exception e) {
            return true;
        }
        return false;
    }

    BlockState parseState(JsonElement e) throws Exception {
        if (e.isJsonPrimitive() && e.getAsJsonPrimitive().isNumber()) {
            return Block.stateById(e.getAsInt());
        }
        return BlockStateParser.parseForBlock(level.holderLookup(Registries.BLOCK), e.getAsString(), false).blockState();
    }

    @SuppressWarnings("unchecked")
    static java.util.List<ItemStack> containerItems(BlockEntity be) {
        for (Class<?> c = be.getClass(); c != null; c = c.getSuperclass()) {
            try {
                var m = c.getDeclaredMethod("getItems");
                m.setAccessible(true);
                return (java.util.List<ItemStack>) m.invoke(be);
            } catch (NoSuchMethodException e) {
                // keep looking up the hierarchy
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        }
        throw new IllegalStateException("no getItems() on " + be.getClass());
    }

    static Object field(Object o, String name) {
        for (Class<?> c = o.getClass(); c != null; c = c.getSuperclass()) {
            try {
                var f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f.get(o);
            } catch (NoSuchFieldException e) {
                // continue up
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        }
        throw new IllegalStateException("no field " + name);
    }

    static void setField(Object o, String name, Object v) {
        for (Class<?> c = o.getClass(); c != null; c = c.getSuperclass()) {
            try {
                var f = c.getDeclaredField(name);
                f.setAccessible(true);
                f.set(o, v);
                return;
            } catch (NoSuchFieldException e) {
                // continue up
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        }
        throw new IllegalStateException("no field " + name);
    }

    static String describeBlockEntity(BlockEntity be) {
        if (be instanceof ComparatorBlockEntity c) {
            return "comparator:" + c.getOutputSignal();
        }
        if (be instanceof PistonMovingBlockEntity p) {
            return "moving:" + Block.getId(p.getMovedState()) + ":" + p.getDirection().getName() + ":" + p.isExtending() + ":"
                + p.isSourcePiston() + ":" + p.getProgress(1.0F);
        }
        if (be instanceof net.minecraft.world.level.block.entity.LecternBlockEntity l) {
            return "lectern:" + l.getPage() + "/" + (int) field(l, "pageCount");
        }
        if (be instanceof net.minecraft.world.level.block.entity.ChiseledBookShelfBlockEntity b) {
            return "bookshelf:" + b.getLastInteractedSlot();
        }
        if (be instanceof net.minecraft.world.level.block.entity.JukeboxBlockEntity j) {
            var item = j.getTheItem();
            boolean playing = j.getSongPlayer().isPlaying();
            return "jukebox:" + (item.isEmpty() ? "" : item.typeHolder().getRegisteredName()) + ":" + playing + ":"
                + (playing ? j.getSongPlayer().getTicksSinceSongStarted() : 0);
        }
        if (be instanceof net.minecraft.world.level.block.entity.SculkSensorBlockEntity sk) {
            var data = sk.getVibrationData();
            var cur = data.getCurrentVibration();
            String ev = cur == null ? "" : cur.gameEvent().unwrapKey().map(k -> k.identifier().getPath()).orElse("?");
            return "sculk:" + sk.getLastVibrationFrequency() + ":" + ev + ":" + (cur == null ? 0 : data.getTravelTimeInTicks());
        }
        if (be instanceof Container c) {
            StringBuilder sb = new StringBuilder("container:");
            for (int i = 0; i < c.getContainerSize(); i++) {
                ItemStack st = c.getItem(i);
                if (!st.isEmpty()) {
                    sb.append(i).append('=').append(st.typeHolder().getRegisteredName()).append('x').append(st.getCount()).append(';');
                }
            }
            return sb.toString();
        }
        return null;
    }

    final class Scenario {
        final String id;
        final JsonObject req;
        final int sx, sy, sz, ticks;
        int ox, oy, oz;
        int step = 0;
        int[] last;
        Map<Long, String> lastBe = new HashMap<>();
        final JsonArray steps = new JsonArray();
        final List<JsonObject> actions = new ArrayList<>();
        long startTime;
        long t0;
        long tickStart;
        /** scenarios with daylight detectors need settled sky light at every tick start */
        final boolean needsLight;

        Scenario(JsonObject req) {
            this.req = req;
            this.id = req.has("id") ? req.get("id").getAsString() : "";
            JsonArray size = req.getAsJsonArray("size");
            this.sx = size.get(0).getAsInt();
            this.sy = size.get(1).getAsInt();
            this.sz = size.get(2).getAsInt();
            if (sx > MAX_XZ || sz > MAX_XZ || sy > MAX_H) throw new IllegalArgumentException("scenario too large");
            this.ticks = req.get("ticks").getAsInt();
            this.needsLight = readsLight(req);
            if (req.has("actions")) {
                for (JsonElement a : req.getAsJsonArray("actions")) actions.add(a.getAsJsonObject());
            }
        }

        BlockPos abs(int x, int y, int z) {
            return new BlockPos(ox + x, oy + y, oz + z);
        }

        BoundingBox clearBox() {
            return new BoundingBox(ox - 16, oy - 16, oz - 16, ox + MAX_XZ + 16, oy + MAX_H + 16, oz + MAX_XZ + 16);
        }

        List<Long> absChunks;

        /** For scenarios with an absolute origin: force-load their chunks, report when they tick. */
        boolean chunksReady() {
            if (!req.has("abs_origin")) return true;
            JsonArray o = req.getAsJsonArray("abs_origin");
            int ax = o.get(0).getAsInt(), az = o.get(2).getAsInt();
            if (absChunks == null) {
                absChunks = new ArrayList<>();
                for (int cx = ((ax - 16) >> 4) - 1; cx <= ((ax + sx + 16) >> 4) + 1; cx++) {
                    for (int cz = ((az - 16) >> 4) - 1; cz <= ((az + sz + 16) >> 4) + 1; cz++) {
                        level.setChunkForced(cx, cz, true);
                        absChunks.add(net.minecraft.world.level.ChunkPos.pack(cx, cz));
                    }
                }
            }
            for (long c : absChunks) {
                if (!level.shouldTickBlocksAt(c) || !level.isPositionTickingWithEntitiesLoaded(c)) return false;
            }
            return true;
        }

        void start(int slot) throws Exception {
            t0 = System.nanoTime();
            int i = slot / GRID, j = slot % GRID;
            JsonArray off = req.has("offset") ? req.getAsJsonArray("offset") : null;
            ox = BASE_X + i * SLOT_SPACING + (off != null ? off.get(0).getAsInt() : 0);
            oy = BASE_Y + (off != null ? off.get(1).getAsInt() : 0);
            oz = BASE_Z + j * SLOT_SPACING + (off != null ? off.get(2).getAsInt() : 0);
            if (req.has("abs_origin")) {
                JsonArray o = req.getAsJsonArray("abs_origin");
                ox = o.get(0).getAsInt();
                oy = o.get(1).getAsInt();
                oz = o.get(2).getAsInt();
            }
            // Clear the slot completely: pending ticks, block events, entities, blocks.
            BoundingBox box = clearBox();
            level.getBlockTicks().clearArea(box);
            level.getFluidTicks().clearArea(box);
            level.clearBlockEvents(box);
            clearTorchToggles(box);
            for (Entity e : level.getEntities((Entity) null, AABB.of(box), e -> !(e instanceof Player))) {
                e.discard();
            }
            BlockState air = net.minecraft.world.level.block.Blocks.AIR.defaultBlockState();
            for (int y = box.minY(); y <= box.maxY(); y++) {
                for (int z = box.minZ(); z <= box.maxZ(); z++) {
                    for (int x = box.minX(); x <= box.maxX(); x++) {
                        BlockPos p = new BlockPos(x, y, z);
                        if (!level.getBlockState(p).isAir()) {
                            level.setBlock(p, air, 2 | 16 | 32 | 256 | 512);
                        }
                    }
                }
            }
            // Place the initial blocks exactly, with no updates and no onPlace.
            for (JsonElement el : req.getAsJsonArray("blocks")) {
                JsonArray b = el.getAsJsonArray();
                BlockPos p = abs(b.get(0).getAsInt(), b.get(1).getAsInt(), b.get(2).getAsInt());
                level.setBlock(p, parseState(b.get(3)), 2 | 16 | 32 | 256 | 512);
            }
            if (req.has("block_entities")) {
                for (JsonElement el : req.getAsJsonArray("block_entities")) {
                    JsonObject o = el.getAsJsonObject();
                    JsonArray pos = o.getAsJsonArray("pos");
                    BlockEntity be = level.getBlockEntity(abs(pos.get(0).getAsInt(), pos.get(1).getAsInt(), pos.get(2).getAsInt()));
                    if (be instanceof ComparatorBlockEntity c && o.has("output")) {
                        c.setOutputSignal(o.get("output").getAsInt());
                    }
                    if (be instanceof net.minecraft.world.level.block.entity.LecternBlockEntity l && o.has("lectern")) {
                        JsonArray lp = o.getAsJsonArray("lectern");
                        int pages = lp.get(1).getAsInt();
                        java.util.List<net.minecraft.server.network.Filterable<String>> content = new ArrayList<>();
                        for (int k = 0; k < pages; k++) content.add(net.minecraft.server.network.Filterable.passThrough("p" + k));
                        ItemStack book = new ItemStack(net.minecraft.world.item.Items.WRITABLE_BOOK);
                        book.set(net.minecraft.core.component.DataComponents.WRITABLE_BOOK_CONTENT, new net.minecraft.world.item.component.WritableBookContent(content));
                        setField(l, "book", book);
                        setField(l, "pageCount", pages);
                        setField(l, "page", Math.max(0, Math.min(lp.get(0).getAsInt(), pages - 1)));
                    }
                    if (be instanceof net.minecraft.world.level.block.entity.ChiseledBookShelfBlockEntity b && o.has("bookshelf")) {
                        setField(b, "lastInteractedSlot", o.get("bookshelf").getAsInt());
                    }
                    if (be instanceof net.minecraft.world.level.block.entity.JukeboxBlockEntity jb && o.has("jukebox")) {
                        var item = net.minecraft.core.registries.BuiltInRegistries.ITEM.getValue(net.minecraft.resources.Identifier.parse(o.get("jukebox").getAsString()));
                        setField(jb, "item", new ItemStack(item));
                        var song = net.minecraft.world.item.JukeboxSong.fromStack(new ItemStack(item));
                        if (song.isPresent()) jb.getSongPlayer().setSongWithoutPlaying(song.get(), 0L);
                    }
                    if (be instanceof net.minecraft.world.level.block.entity.DecoratedPotBlockEntity pot && o.has("items")) {
                        // single-item container: setTheItem writes the field (no setChanged)
                        for (JsonElement it : o.getAsJsonArray("items")) {
                            JsonArray a = it.getAsJsonArray();
                            var item = net.minecraft.core.registries.BuiltInRegistries.ITEM.getValue(
                                net.minecraft.resources.Identifier.parse(a.get(1).getAsString()));
                            if (a.get(0).getAsInt() == 0) pot.setTheItem(new ItemStack(item, a.get(2).getAsInt()));
                        }
                    } else if (be instanceof Container c && o.has("items")) {
                        // Write contents directly (like loading a schematic): setItem() would call
                        // setChanged() and notify comparators, which a paste does not do.
                        java.util.List<ItemStack> list = containerItems(be);
                        for (JsonElement it : o.getAsJsonArray("items")) {
                            JsonArray a = it.getAsJsonArray();
                            var item = net.minecraft.core.registries.BuiltInRegistries.ITEM.getValue(
                                net.minecraft.resources.Identifier.parse(a.get(1).getAsString()));
                            list.set(a.get(0).getAsInt(), new ItemStack(item, a.get(2).getAsInt()));
                        }
                    }
                }
            }
            startTime = level.getGameTime();
            last = new int[sx * sy * sz];
            JsonArray initial = new JsonArray();
            for (int y = 0; y < sy; y++) for (int z = 0; z < sz; z++) for (int x = 0; x < sx; x++) {
                int sid = Block.getId(level.getBlockState(abs(x, y, z)));
                last[(y * sz + z) * sx + x] = sid;
                if (sid != 0) {
                    JsonArray c = new JsonArray();
                    c.add(x); c.add(y); c.add(z); c.add(sid);
                    initial.add(c);
                }
            }
            JsonObject st = new JsonObject();
            st.addProperty("t", 0);
            st.add("tick", initial);
            st.add("be", diffBe());
            steps.add(st);
            runActions();
        }

        JsonArray diffBlocks() {
            JsonArray changes = new JsonArray();
            for (int y = 0; y < sy; y++) for (int z = 0; z < sz; z++) for (int x = 0; x < sx; x++) {
                int idx = (y * sz + z) * sx + x;
                int sid = Block.getId(level.getBlockState(abs(x, y, z)));
                if (sid != last[idx]) {
                    last[idx] = sid;
                    JsonArray c = new JsonArray();
                    c.add(x); c.add(y); c.add(z); c.add(sid);
                    changes.add(c);
                }
            }
            return changes;
        }

        JsonArray diffBe() {
            Map<Long, String> now = new HashMap<>();
            for (int y = 0; y < sy; y++) for (int z = 0; z < sz; z++) for (int x = 0; x < sx; x++) {
                BlockPos p = abs(x, y, z);
                if (level.getBlockState(p).hasBlockEntity()) {
                    BlockEntity be = level.getBlockEntity(p);
                    if (be != null) {
                        String d = describeBlockEntity(be);
                        if (d != null) now.put(BlockPos.asLong(x, y, z), d);
                    }
                }
            }
            JsonArray out = new JsonArray();
            for (var e : now.entrySet()) {
                if (!e.getValue().equals(lastBe.get(e.getKey()))) {
                    BlockPos p = BlockPos.of(e.getKey());
                    JsonArray c = new JsonArray();
                    c.add(p.getX()); c.add(p.getY()); c.add(p.getZ()); c.add(e.getValue());
                    out.add(c);
                }
            }
            for (var k : lastBe.keySet()) {
                if (!now.containsKey(k)) {
                    BlockPos p = BlockPos.of(k);
                    JsonArray c = new JsonArray();
                    c.add(p.getX()); c.add(p.getY()); c.add(p.getZ()); c.add((String) null);
                    out.add(c);
                }
            }
            lastBe = now;
            return out;
        }

        void recordTick() {
            if (req.has("record") && !req.get("record").getAsBoolean()) {
                if (step == 1) tickStart = System.nanoTime();
                return;
            }
            JsonObject st = new JsonObject();
            st.addProperty("t", step);
            st.add("tick", diffBlocks());
            st.add("be", diffBe());
            steps.add(st);
        }

        void runActions() throws Exception {
            boolean any = false;
            for (JsonObject a : actions) {
                if (a.get("at").getAsInt() != step) continue;
                any = true;
                JsonArray pos = a.getAsJsonArray("pos");
                BlockPos p = abs(pos.get(0).getAsInt(), pos.get(1).getAsInt(), pos.get(2).getAsInt());
                String type = a.get("type").getAsString();
                switch (type) {
                    case "use" -> useBlock(p);
                    case "press" -> {
                        BlockState s = level.getBlockState(p);
                        if (s.getBlock() instanceof ButtonBlock b) b.press(s, level, p, null);
                    }
                    case "setblock" -> {
                        int flags = a.has("flags") ? a.get("flags").getAsInt() : 3;
                        level.setBlock(p, parseState(a.get("state")), flags);
                    }
                    case "destroy" -> level.destroyBlock(p, false);
                    case "turn_page" -> {
                        if (level.getBlockEntity(p) instanceof net.minecraft.world.level.block.entity.LecternBlockEntity l) {
                            var m = l.getClass().getDeclaredMethod("setPage", int.class);
                            m.setAccessible(true);
                            m.invoke(l, a.get("page").getAsInt());
                        }
                    }
                    case "player_place" -> {
                        // a player placing the block's item onto the block below (face UP), using the
                        // game's own getStateForPlacement (player looks south, yaw 0)
                        Block block = parseState(a.get("state")).getBlock();
                        BlockHitResult hit = new BlockHitResult(Vec3.atCenterOf(p), Direction.UP, p, false);
                        var ctx = new net.minecraft.world.item.context.BlockPlaceContext(player, InteractionHand.MAIN_HAND, new ItemStack(block.asItem()), hit);
                        BlockState st = block.getStateForPlacement(ctx);
                        if (st != null && level.setBlock(p, st, 11)) {
                            BlockState placed = level.getBlockState(p);
                            if (placed.is(st.getBlock())) placed.getBlock().setPlacedBy(level, p, placed, null, ItemStack.EMPTY);
                            level.gameEvent(net.minecraft.world.level.gameevent.GameEvent.BLOCK_PLACE, p,
                                net.minecraft.world.level.gameevent.GameEvent.Context.of(player, placed));
                        }
                    }
                    case "place" -> {
                        // BlockItem.place: setBlock(11), then setPlacedBy if the block is still there
                        BlockState st = parseState(a.get("state"));
                        if (level.setBlock(p, st, 11)) {
                            BlockState placed = level.getBlockState(p);
                            if (placed.is(st.getBlock())) {
                                placed.getBlock().setPlacedBy(level, p, placed, null, ItemStack.EMPTY);
                            }
                            // BlockItem.place ends with the BLOCK_PLACE game event
                            level.gameEvent(net.minecraft.world.level.gameevent.GameEvent.BLOCK_PLACE, p,
                                net.minecraft.world.level.gameevent.GameEvent.Context.of(player, placed));
                        }
                    }
                    default -> throw new IllegalArgumentException("unknown action " + type);
                }
            }
            if (any && !(req.has("record") && !req.get("record").getAsBoolean())) {
                JsonObject last = steps.get(steps.size() - 1).getAsJsonObject();
                last.add("act", diffBlocks());
                last.add("act_be", diffBe());
            }
        }

        void useBlock(BlockPos p) {
            BlockState state = level.getBlockState(p);
            BlockHitResult hit = new BlockHitResult(Vec3.atCenterOf(p), Direction.UP, p, false);
            InteractionResult r = state.useItemOn(player.getItemInHand(InteractionHand.MAIN_HAND), level, player, InteractionHand.MAIN_HAND, hit);
            if (!r.consumesAction()) {
                if (r instanceof InteractionResult.TryEmptyHandInteraction) {
                    state.useWithoutItem(level, player, hit);
                }
            }
        }

        void finish() {
            JsonObject o = new JsonObject();
            o.addProperty("id", id);
            o.addProperty("ok", true);
            JsonArray origin = new JsonArray();
            origin.add(ox); origin.add(oy); origin.add(oz);
            o.add("origin", origin);
            o.addProperty("start_time", startTime);
            o.add("steps", steps);
            o.addProperty("wall_ms", (System.nanoTime() - t0) / 1e6);
            if (tickStart != 0) o.addProperty("tick_ms", (System.nanoTime() - tickStart) / 1e6);
            Main.respond(gson.toJson(o));
        }
    }
}
