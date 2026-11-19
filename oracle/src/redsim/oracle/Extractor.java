package redsim.oracle;

import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashSet;
import java.util.Set;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SupportType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.material.FluidState;

/**
 * Dumps the facts the simulator needs about every block state, computed by the
 * real game code: ids, properties, redstone/piston/support predicates, block class
 * hierarchy, which behaviour methods each class overrides, and block tags.
 */
final class Extractor {
    static final String[] HOOKS = {
        "updateShape", "neighborChanged", "tick", "randomTick", "onPlace", "affectNeighborsAfterRemoval", "getSignal", "getDirectSignal",
        "isSignalSource", "ownSignal", "hasAnalogOutputSignal", "getAnalogOutputSignal", "triggerEvent", "useWithoutItem", "useItemOn",
        "getTicker", "newBlockEntity", "entityInside", "stepOn", "onProjectileHit", "updateIndirectNeighbourShapes", "canSurvive",
        "shouldRedstoneWireConnectTo", "getStateForPlacement", "setPlacedBy", "playerWillDestroy", "onExplosionHit", "attack",
        "fallOn", "updateEntityMovementAfterFallOn", "isRandomlyTicking", "getCollisionShape", "getBlockSupportShape"
    };

    static void extract(MinecraftServer server, Path out) throws Exception {
        JsonObject root = new JsonObject();
        root.addProperty("version", net.minecraft.SharedConstants.getCurrentVersion().name());
        root.addProperty("data_version", net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());
        java.util.IdentityHashMap<Property<?>, String> propNames = new java.util.IdentityHashMap<>();
        for (java.lang.reflect.Field f : net.minecraft.world.level.block.state.properties.BlockStateProperties.class.getFields()) {
            if (java.lang.reflect.Modifier.isStatic(f.getModifiers()) && Property.class.isAssignableFrom(f.getType())) {
                propNames.putIfAbsent((Property<?>) f.get(null), f.getName());
            }
        }
        JsonArray blocks = new JsonArray();
        // light: distinct face occlusion shapes (LightEngine.getOcclusionShape), id 0 = empty
        java.util.LinkedHashMap<String, Integer> faceIds = new java.util.LinkedHashMap<>();
        java.util.ArrayList<net.minecraft.world.phys.shapes.VoxelShape> faceShapes = new java.util.ArrayList<>();
        faceIds.put("", 0);
        faceShapes.add(net.minecraft.world.phys.shapes.Shapes.empty());
        java.util.LinkedHashMap<String, Integer> shapeIds = new java.util.LinkedHashMap<>();
        JsonArray shapes = new JsonArray();
        EmptyBlockGetter g = EmptyBlockGetter.INSTANCE;
        BlockPos o = BlockPos.ZERO;
        for (Block block : BuiltInRegistries.BLOCK) {
            JsonObject b = new JsonObject();
            b.addProperty("name", BuiltInRegistries.BLOCK.getKey(block).toString());
            JsonArray classes = new JsonArray();
            Set<String> hooks = new LinkedHashSet<>();
            for (Class<?> c = block.getClass(); c != null && c != BlockBehaviour.class && c != Object.class; c = c.getSuperclass()) {
                if (c.isAnonymousClass()) continue;
                classes.add(c.getSimpleName());
                if (c == Block.class) continue;
                for (Method m : c.getDeclaredMethods()) {
                    for (String h : HOOKS) if (m.getName().equals(h)) hooks.add(h);
                }
            }
            b.add("classes", classes);
            JsonArray hk = new JsonArray();
            hooks.forEach(hk::add);
            b.add("hooks", hk);
            b.addProperty("has_dynamic_shape", block.hasDynamicShape());
            b.add("fields", dumpFields(block));
            if (block instanceof net.minecraft.world.level.block.EntityBlock eb) {
                try {
                    var be = eb.newBlockEntity(BlockPos.ZERO, block.defaultBlockState());
                    if (be instanceof net.minecraft.world.Container c) {
                        b.addProperty("container_size", c.getContainerSize());
                        b.addProperty("container_max_stack", c.getMaxStackSize());
                    }
                    if (be != null) b.addProperty("block_entity", be.getType().builtInRegistryHolder().key().identifier().toString());
                } catch (Throwable ignored) {
                }
            }
            var inst = block.defaultBlockState().instrument();
            b.addProperty("instrument", inst.getSerializedName());
            b.addProperty("instrument_above", inst.worksAboveNoteBlock());
            JsonArray tags = new JsonArray();
            Holder<Block> holder = BuiltInRegistries.BLOCK.wrapAsHolder(block);
            holder.tags().map(t -> t.location().toString()).sorted().forEach(tags::add);
            b.add("tags", tags);
            JsonArray props = new JsonArray();
            for (Property<?> p : block.getStateDefinition().getProperties()) {
                JsonObject po = new JsonObject();
                po.addProperty("name", p.getName());
                String canon = propNames.get(p);
                if (canon == null) {
                    for (Class<?> c = block.getClass(); c != null && canon == null && c != Object.class; c = c.getSuperclass()) {
                        for (java.lang.reflect.Field f : c.getDeclaredFields()) {
                            if (java.lang.reflect.Modifier.isStatic(f.getModifiers()) && Property.class.isAssignableFrom(f.getType())) {
                                f.setAccessible(true);
                                if (f.get(null) == p) { canon = c.getSimpleName() + "." + f.getName(); break; }
                            }
                        }
                    }
                }
                po.addProperty("field", canon == null ? "" : canon);
                JsonArray vals = new JsonArray();
                for (Object v : p.getPossibleValues()) vals.add(nameOf(p, v));
                po.add("values", vals);
                props.add(po);
            }
            b.add("properties", props);
            b.addProperty("default", Block.getId(block.defaultBlockState()));
            JsonArray states = new JsonArray();
            for (BlockState s : block.getStateDefinition().getPossibleStates()) {
                JsonObject so = new JsonObject();
                so.addProperty("id", Block.getId(s));
                JsonArray pv = new JsonArray();
                for (Property<?> p : block.getStateDefinition().getProperties()) pv.add(nameOf(p, s.getValue(p)));
                so.add("values", pv);
                so.addProperty("conductor", s.isRedstoneConductor(g, o));
                so.addProperty("signal_source", s.isSignalSource());
                so.addProperty("analog", s.hasAnalogOutputSignal());
                so.addProperty("air", s.isAir());
                so.addProperty("replaceable", s.canBeReplaced());
                so.addProperty("has_be", s.hasBlockEntity());
                so.addProperty("solid", s.isSolid());
                so.addProperty("full_collision", s.isCollisionShapeFullBlock(g, o));
                so.addProperty("solid_render", s.isSolidRender());
                so.addProperty("push", s.getPistonPushReaction().name());
                so.addProperty("destroy_speed", s.getDestroySpeed(g, o));
                so.addProperty("light", s.getLightEmission());
                so.addProperty("ticks_randomly", s.isRandomlyTicking());
                FluidState f = s.getFluidState();
                so.addProperty("fluid", f.isEmpty() ? "" : BuiltInRegistries.FLUID.getKey(f.getType()).toString() + ":" + f.getAmount());
                so.addProperty("sturdy_full", faces(s, SupportType.FULL));
                so.addProperty("sturdy_center", faces(s, SupportType.CENTER));
                so.addProperty("sturdy_rigid", faces(s, SupportType.RIGID));
                so.addProperty("collision_empty", s.getCollisionShape(g, o).isEmpty());
                so.addProperty("shape_full", Block.isShapeFullBlock(s.getShape(g, o)));
                JsonArray boxes = new JsonArray();
                for (net.minecraft.world.phys.AABB bb : s.getCollisionShape(g, o).toAabbs()) {
                    JsonArray a = new JsonArray();
                    a.add(bb.minX); a.add(bb.minY); a.add(bb.minZ); a.add(bb.maxX); a.add(bb.maxY); a.add(bb.maxZ);
                    boxes.add(a);
                }
                String key = boxes.toString();
                Integer sid = shapeIds.get(key);
                if (sid == null) { sid = shapes.size(); shapeIds.put(key, sid); shapes.add(boxes); }
                so.addProperty("collision", sid);
                JsonArray tr = new JsonArray();
                tr.add(Block.getId(s.rotate(net.minecraft.world.level.block.Rotation.CLOCKWISE_90)));
                tr.add(Block.getId(s.rotate(net.minecraft.world.level.block.Rotation.CLOCKWISE_180)));
                tr.add(Block.getId(s.rotate(net.minecraft.world.level.block.Rotation.COUNTERCLOCKWISE_90)));
                tr.add(Block.getId(s.mirror(net.minecraft.world.level.block.Mirror.LEFT_RIGHT)));
                tr.add(Block.getId(s.mirror(net.minecraft.world.level.block.Mirror.FRONT_BACK)));
                so.add("transform", tr);
                so.addProperty("light_dampening", s.getLightDampening());
                JsonArray fo = new JsonArray();
                for (Direction d : Direction.values()) {
                    var fs = net.minecraft.world.level.lighting.LightEngine.getOcclusionShape(s, d);
                    String fk = fs.isEmpty() ? "" : fs.toAabbs().toString();
                    Integer fid = faceIds.get(fk);
                    if (fid == null) { fid = faceShapes.size(); faceIds.put(fk, fid); faceShapes.add(fs); }
                    fo.add(fid);
                }
                so.add("face_occ", fo);
                states.add(so);
            }
            b.add("states", states);
            blocks.add(b);
        }
        root.add("blocks", blocks);
        root.add("collision_shapes", shapes);
        JsonObject items = new JsonObject();
        for (net.minecraft.world.item.Item item : BuiltInRegistries.ITEM) {
            items.addProperty(BuiltInRegistries.ITEM.getKey(item).toString(), item.getDefaultMaxStackSize());
        }
        root.add("item_max_stack", items);
        JsonObject songs = new JsonObject();
        for (net.minecraft.world.item.Item item : BuiltInRegistries.ITEM) {
            var stack = new net.minecraft.world.item.ItemStack(item);
            var song = net.minecraft.world.item.JukeboxSong.fromStack(stack);
            if (song.isPresent()) {
                var v = song.get().value();
                JsonArray a = new JsonArray();
                a.add(v.comparatorOutput());
                a.add(v.lengthInTicks());
                songs.add(BuiltInRegistries.ITEM.getKey(item).toString(), a);
            }
        }
        root.add("jukebox_songs", songs);
        // Shapes.faceShapeOccludes over all pairs of face occlusion shapes (symmetric)
        JsonArray occ = new JsonArray();
        for (int i = 0; i < faceShapes.size(); i++) {
            for (int j = i; j < faceShapes.size(); j++) {
                if (net.minecraft.world.phys.shapes.Shapes.faceShapeOccludes(faceShapes.get(i), faceShapes.get(j))) {
                    JsonArray pr = new JsonArray(); pr.add(i); pr.add(j); occ.add(pr);
                }
            }
        }
        root.addProperty("face_shape_count", faceShapes.size());
        root.add("face_occludes", occ);
        // overworld day clock: per day tick, the daylight detector's cos(sun angle) term (as float
        // bits) and the sky darkening, sampled from the game's own environment attributes
        var level = server.overworld();
        var clock = level.dimensionType().defaultClock().orElseThrow();
        var clocks = server.clockManager();
        long savedTicks = clocks.getInstance(clock).totalTicks();
        StringBuilder day = new StringBuilder();
        BlockPos probe = new BlockPos(0, 100, 0);
        for (int t = 0; t < 24000; t++) {
            clocks.setTotalTicks(clock, t);
            level.environmentAttributes().invalidateTickCache();
            float angle = level.environmentAttributes().getValue(net.minecraft.world.attribute.EnvironmentAttributes.SUN_ANGLE, probe);
            float skyLevel = level.environmentAttributes().getDimensionValue(net.minecraft.world.attribute.EnvironmentAttributes.SKY_LIGHT_LEVEL);
            int darken = (int)(15.0F - skyLevel);
            // DaylightDetectorBlock.updateSignalStrength
            float sunAngle = angle * 0.017453292F;
            float offset = sunAngle < 3.1415927F ? 0.0F : 6.2831855F;
            sunAngle += (offset - sunAngle) * 0.2F;
            float c = net.minecraft.util.Mth.cos(sunAngle);
            day.append(String.format("%08x%x", Float.floatToRawIntBits(c), darken));
        }
        clocks.setTotalTicks(clock, savedTicks);
        level.environmentAttributes().invalidateTickCache();
        level.updateSkyBrightness();
        root.addProperty("daylight", day.toString());
        Files.createDirectories(out.toAbsolutePath().getParent());
        Files.writeString(out, new GsonBuilder().disableHtmlEscaping().create().toJson(root));
    }

    /** Instance fields of primitive/enum/record type declared by block classes (e.g. button press time). */
    static JsonObject dumpFields(Block block) {
        JsonObject o = new JsonObject();
        for (Class<?> c = block.getClass(); c != null && c != BlockBehaviour.class; c = c.getSuperclass()) {
            for (java.lang.reflect.Field f : c.getDeclaredFields()) {
                if (java.lang.reflect.Modifier.isStatic(f.getModifiers())) continue;
                try {
                    f.setAccessible(true);
                    Object v = f.get(block);
                    String key = c.getSimpleName() + "." + f.getName();
                    if (v instanceof Number n) o.addProperty(key, n);
                    else if (v instanceof Boolean bo) o.addProperty(key, bo);
                    else if (v instanceof Enum<?> e) o.addProperty(key, e.name());
                    else if (v instanceof String str) o.addProperty(key, str);
                    else if (v instanceof Record r) {
                        JsonObject ro = new JsonObject();
                        for (var rc : r.getClass().getRecordComponents()) {
                            Object rv = rc.getAccessor().invoke(r);
                            if (rv instanceof Number n) ro.addProperty(rc.getName(), n);
                            else if (rv instanceof Boolean bo) ro.addProperty(rc.getName(), bo);
                            else if (rv instanceof Enum<?> e) ro.addProperty(rc.getName(), e.name());
                            else if (rv instanceof String str) ro.addProperty(rc.getName(), str);
                        }
                        o.add(key, ro);
                    }
                } catch (Throwable ignored) {
                }
            }
        }
        return o;
    }

    static int faces(BlockState s, SupportType t) {
        int mask = 0;
        for (Direction d : Direction.values()) {
            if (s.isFaceSturdy(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, d, t)) mask |= 1 << d.ordinal();
        }
        return mask;
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static String nameOf(Property p, Object v) {
        return p.getName((Comparable) v);
    }
}
