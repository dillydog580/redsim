package redsim.oracle;

import com.mojang.authlib.services.ServicesKeySet;
import com.mojang.serialization.Lifecycle;
import java.net.Proxy;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.UUID;
import java.util.function.BooleanSupplier;
import net.minecraft.SystemReport;
import net.minecraft.commands.Commands;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.Registries;
import net.minecraft.gizmos.GizmoCollector;
import net.minecraft.gizmos.Gizmos;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.Services;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.WorldStem;
import net.minecraft.server.level.progress.LoggingLevelLoadListener;
import net.minecraft.server.notifications.EmptyNotificationService;
import net.minecraft.server.notifications.NotificationManager;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.permissions.PermissionSet;
import net.minecraft.server.players.NameAndId;
import net.minecraft.server.players.PlayerList;
import net.minecraft.server.players.ProfileResolver;
import net.minecraft.server.players.UserNameToIdResolver;
import net.minecraft.util.Util;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.util.debugchart.LocalSampleLogger;
import net.minecraft.util.debugchart.SampleLogger;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.level.DataPackConfig;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.dimension.LevelStem;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.WorldDimensions;
import net.minecraft.world.level.levelgen.WorldGenSettings;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.storage.LevelDataAndDimensions;
import net.minecraft.world.level.storage.LevelStorageSource;
import net.minecraft.world.level.storage.PrimaryLevelData;

/**
 * Minimal headless server (no network) running vanilla 26.3 with default feature
 * flags (no Redstone Experiments). Ticks run back to back with no sleeping; the
 * {@link Harness} hooks in at the end of every server tick.
 */
public final class OracleServer extends MinecraftServer {
    private static final Services NO_SERVICES = new Services(null, ServicesKeySet.EMPTY, null, new NoNames(), new NoProfiles());
    private final LocalSampleLogger sampleLogger = new LocalSampleLogger(4);
    public Harness harness;

    public static OracleServer create(Thread thread, LevelStorageSource.LevelStorageAccess storage, PackRepository packs) {
        packs.reload();
        WorldDataConfiguration config = new WorldDataConfiguration(new DataPackConfig(List.of("vanilla"), List.of()), FeatureFlags.DEFAULT_FLAGS);
        LevelSettings settings = new LevelSettings("redsim oracle", GameType.CREATIVE, LevelSettings.DifficultySettings.DEFAULT, true, config);
        WorldLoader.PackConfig packConfig = new WorldLoader.PackConfig(packs, config, false, true);
        WorldLoader.InitConfig initConfig = new WorldLoader.InitConfig(packConfig, Commands.CommandSelection.DEDICATED, LevelBasedPermissionSet.OWNER);
        try {
            WorldStem stem = Util.<WorldStem>blockUntilDone(executor -> WorldLoader.load(initConfig, context -> {
                Registry<LevelStem> noDatapackDimensions = new MappedRegistry<>(Registries.LEVEL_STEM, Lifecycle.stable()).freeze();
                WorldDimensions dims = context.datapackWorldRegistries().lookupOrThrow(Registries.WORLD_PRESET)
                    .getOrThrow(WorldPresets.FLAT_ALL_DIMENSIONS).value().createWorldDimensions();
                WorldDimensions.Complete complete = dims.bake(noDatapackDimensions);
                PrimaryLevelData data = new PrimaryLevelData(settings, complete.specialWorldProperty(), complete.lifecycle());
                return new WorldLoader.DataLoadOutput<>(
                    new LevelDataAndDimensions.WorldDataAndGenSettings(data, new WorldGenSettings(new WorldOptions(0L, false, false), dims)),
                    complete.dimensionsRegistryAccess());
            }, WorldStem::new, Util.backgroundExecutor(), executor)).get();
            return new OracleServer(thread, storage, packs, stem);
        } catch (Exception e) {
            throw new RuntimeException("failed to create oracle server", e);
        }
    }

    private OracleServer(Thread thread, LevelStorageSource.LevelStorageAccess storage, PackRepository packs, WorldStem stem) {
        super(thread, storage, packs, stem, Optional.of(new GameRules(FeatureFlags.DEFAULT_FLAGS)), Proxy.NO_PROXY,
            DataFixers.getDataFixer(), NO_SERVICES, LoggingLevelLoadListener.forDedicatedServer(), false, new NotificationManager());
    }

    @Override
    protected boolean initServer() {
        this.setPlayerList(new PlayerList(this, this.registries(), this.playerDataStorage, new EmptyNotificationService()) {});
        Gizmos.withCollector(GizmoCollector.NOOP);
        this.loadLevel();
        GameRules rules = this.getGameRules();
        rules.set(GameRules.RANDOM_TICK_SPEED, 0, this);
        rules.set(GameRules.ADVANCE_TIME, false, this);
        rules.set(GameRules.ADVANCE_WEATHER, false, this);
        rules.set(GameRules.SPAWN_MOBS, false, this);
        rules.set(GameRules.SPAWN_MONSTERS, false, this);
        rules.set(GameRules.SPAWN_PATROLS, false, this);
        rules.set(GameRules.SPAWN_PHANTOMS, false, this);
        rules.set(GameRules.SPAWN_WANDERING_TRADERS, false, this);
        rules.set(GameRules.SPAWN_WARDENS, false, this);
        rules.set(GameRules.BLOCK_DROPS, !Main.slots, this);
        this.harness = new Harness(this);
        this.harness.init();
        return true;
    }

    @Override
    protected void tickServer(BooleanSupplier haveTime) {
        if (this.harness != null) this.harness.settleLight();
        super.tickServer(haveTime);
        this.harness.afterTick();
    }

    @Override
    protected void waitUntilNextTick() {
        this.runAllTasks();
        if (this.harness != null) {
            this.harness.idleWait();
        }
    }

    @Override
    protected void onServerExit() {
        super.onServerExit();
        System.exit(0);
    }

    public LevelStorageSource.LevelStorageAccess storageSource() { return this.storageSource; }

    @Override protected SampleLogger getTickTimeLogger() { return this.sampleLogger; }
    @Override public boolean isTickTimeLoggingEnabled() { return false; }
    @Override public SystemReport fillServerSystemReport(SystemReport r) { r.setDetail("Type", "redsim oracle"); return r; }
    @Override public boolean isHardcore() { return false; }
    @Override public LevelBasedPermissionSet operatorUserPermissions() { return LevelBasedPermissionSet.ALL; }
    @Override public PermissionSet getFunctionCompilationPermissions() { return LevelBasedPermissionSet.OWNER; }
    @Override public boolean shouldRconBroadcast() { return false; }
    @Override public boolean isDedicatedServer() { return false; }
    @Override public int getRateLimitPacketsPerSecond() { return 0; }
    @Override public int getCommandSpamThresholdSeconds() { return 0; }
    @Override public int getChatSpamThresholdSeconds() { return 0; }
    @Override public boolean useNativeTransport() { return false; }
    @Override public boolean isPublished() { return false; }
    @Override public boolean shouldInformAdmins() { return false; }
    @Override public boolean isSingleplayerOwner(NameAndId nameAndId) { return false; }
    @Override public int getMaxPlayers() { return 1; }

    private static final class NoProfiles implements ProfileResolver {
        @Override public Optional<com.mojang.authlib.GameProfile> fetchByName(String name) { return Optional.empty(); }
        @Override public Optional<com.mojang.authlib.GameProfile> fetchById(UUID id) { return Optional.empty(); }
    }

    private static final class NoNames implements UserNameToIdResolver {
        private final List<NameAndId> saved = new ArrayList<>();
        @Override public void add(NameAndId n) { saved.add(n); }
        @Override public Optional<NameAndId> get(String name) {
            return saved.stream().filter(e -> e.name().equals(name)).findFirst().or(() -> Optional.of(NameAndId.createOffline(name)));
        }
        @Override public Optional<NameAndId> get(UUID id) { return saved.stream().filter(e -> e.id().equals(id)).findFirst(); }
        @Override public void resolveOfflineUsers(boolean value) {}
        @Override public void save() {}
    }
}
