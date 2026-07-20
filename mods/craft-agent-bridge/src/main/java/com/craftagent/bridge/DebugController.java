package com.craftagent.bridge;

import com.google.gson.JsonObject;
import java.util.Optional;
import java.util.Set;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntitySpawnRequest;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.Mob;
import net.minecraft.world.entity.monster.Monster;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.npc.villager.Villager;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.trading.ItemCost;
import net.minecraft.world.item.trading.MerchantOffer;
import net.minecraft.world.item.trading.MerchantOffers;
import net.minecraft.world.MenuProvider;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.ServerLevelAccessor;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.level.ItemLike;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

public class DebugController {

    public static JsonObject actDebugSpawn(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        EntitySpawnRequest spawnReq;
        String ent = req.has("entity") ? req.get("entity").getAsString().toLowerCase() : "zombie";
        double fx = player.getX() + player.getLookAngle().x * 3.0;
        double fz = player.getZ() + player.getLookAngle().z * 3.0;
        double fy = player.getY() + 1.0;
        if (ent.equals("item")) {
            String itemId = req.has("item") ? req.get("item").getAsString() : "minecraft:oak_log";
            int num = req.has("num") ? req.get("num").getAsInt() : 1;
            Optional holder = BuiltInRegistries.ITEM.get(Identifier.fromNamespaceAndPath((String)(itemId.contains(":") ? itemId.split(":")[0] : "minecraft"), (String)(itemId.contains(":") ? itemId.split(":")[1] : itemId)));
            if (holder.isEmpty()) {
                o.addProperty("detail", "debug_spawn unknown item: " + itemId);
                return o;
            }
            ItemStack stack = new ItemStack((ItemLike)((net.minecraft.core.Holder.Reference)holder.get()).value(), num);
            ItemEntity ie = new ItemEntity((net.minecraft.world.level.Level)level, fx, fy + 1.0, fz, stack);
            level.addFreshEntity((Entity)ie);
            o.addProperty("detail", "debug_spawn item " + itemId + " x" + num);
            return o;
        }
        Optional eth = BuiltInRegistries.ENTITY_TYPE.get(Identifier.fromNamespaceAndPath((String)"minecraft", (String)ent));
        if (eth.isEmpty()) {
            o.addProperty("detail", "debug_spawn unknown entity: " + ent);
            return o;
        }
        EntityType et = (EntityType)((net.minecraft.core.Holder.Reference)eth.get()).value();
        Entity e2 = et.spawn(level, null, BlockPos.containing((double)fx, (double)fy, (double)fz), EntitySpawnReason.COMMAND, true, false);
        if (e2 == null && (e2 = et.create((net.minecraft.world.level.Level)level, spawnReq = new EntitySpawnRequest(EntitySpawnReason.COMMAND, true))) != null) {
            e2.setPos(fx, fy, fz);
            level.addFreshEntity(e2);
        }
        if (e2 == null) {
            o.addProperty("detail", "debug_spawn failed to create: " + ent);
            return o;
        }
        if (e2 instanceof Mob) {
            Mob mob = (Mob)e2;
            mob.finalizeSpawn((ServerLevelAccessor)level, level.getCurrentDifficultyAt(BlockPos.containing((double)fx, (double)fy, (double)fz)), EntitySpawnReason.COMMAND, null);
            mob.setPersistenceRequired();
            mob.setNoAi(false);
            mob.addEffect(new MobEffectInstance(MobEffects.FIRE_RESISTANCE, 999999, 0, false, false));
        }
        if (e2 instanceof LivingEntity && !(e2 instanceof Villager)) {
            e2.startRiding((Entity)player, false, false);
        }
        if (e2 instanceof Villager) {
            String prof;
            Villager v = (Villager)e2;
            String string = prof = req.has("profession") ? req.get("profession").getAsString().toLowerCase() : "";
            if (!prof.isEmpty() && !prof.equals("none")) {
                MerchantOffers offers = new MerchantOffers();
                offers.add(new MerchantOffer(new ItemCost((ItemLike)Items.EMERALD, 1), new ItemStack((ItemLike)Items.BOOK, 1), 5, 5, 0.05f));
                offers.add(new MerchantOffer(new ItemCost((ItemLike)Items.EMERALD, 1), new ItemStack((ItemLike)Items.WHEAT, 4), 5, 5, 0.05f));
                v.setOffers(offers);
                player.getInventory().add(new ItemStack((ItemLike)Items.EMERALD, 16));
                player.containerMenu.broadcastChanges();
                o.addProperty("detail", "debug_spawn villager(profession=" + prof + ") with " + offers.size() + " injected offers");
            }
        }
        int nearbyCount = level.getEntities((Entity)player, AABB.ofSize((Vec3)player.position(), (double)32.0, (double)32.0, (double)32.0)).size();
        StringBuilder nearby = new StringBuilder();
        for (Entity ne : level.getEntities((Entity)player, AABB.ofSize((Vec3)player.position(), (double)32.0, (double)32.0, (double)32.0))) {
            if (ne == player) continue;
            nearby.append(BuiltInRegistries.ENTITY_TYPE.getKey(ne.getType()).toString()).append("(ride=").append(ne.isPassenger()).append(")@").append(String.format("%.1f,%.1f,%.1f", ne.getX(), ne.getY(), ne.getZ())).append("; ");
        }
        o.addProperty("detail", "debug_spawn " + ent + " at " + String.format("%.1f,%.1f,%.1f", e2.getX(), e2.getY(), e2.getZ()) + " | riding=" + e2.isPassenger() + " | nearbyEntities=" + nearbyCount + " | [" + String.valueOf(nearby) + "]");
        return o;
    }

    public static JsonObject actDebugGive(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String itemId = req.has("item") ? req.get("item").getAsString() : "minecraft:oak_log";
        int num = req.has("num") ? req.get("num").getAsInt() : 1;
        Optional holder = BuiltInRegistries.ITEM.get(Identifier.fromNamespaceAndPath((String)(itemId.contains(":") ? itemId.split(":")[0] : "minecraft"), (String)(itemId.contains(":") ? itemId.split(":")[1] : itemId)));
        if (holder.isEmpty()) {
            o.addProperty("detail", "debug_give unknown item: " + itemId);
            return o;
        }
        player.getInventory().add(new ItemStack((ItemLike)((net.minecraft.core.Holder.Reference)holder.get()).value(), num));
        player.containerMenu.broadcastChanges();
        o.addProperty("detail", "debug_give " + itemId + " x" + num);
        return o;
    }

    public static JsonObject actDebugDamage(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        float amt = req.has("amount") ? req.get("amount").getAsFloat() : 5.0f;
        float newHp = Math.max(1.0f, player.getHealth() - amt);
        player.setHealth(newHp);
        o.addProperty("detail", "debug_damage " + amt + " -> hp=" + newHp);
        return o;
    }

    public static JsonObject actDebugHeal(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        float amt = req.has("amount") ? req.get("amount").getAsFloat() : 20.0f;
        float newHp = Math.min(20.0f, player.getHealth() + amt);
        player.setHealth(newHp);
        o.addProperty("detail", "debug_heal " + amt + " -> hp=" + newHp);
        return o;
    }

    public static JsonObject actDebugClear(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        player.getInventory().clearContent();
        player.containerMenu.broadcastChanges();
        AABB global = AABB.ofSize((Vec3)new Vec3(0.0, 64.0, 0.0), (double)100000.0, (double)100000.0, (double)100000.0);
        for (Entity e3 : level.getEntities(null, global)) {
            if (e3 instanceof ItemEntity) {
                e3.discard();
            }
            if (!(e3 instanceof Monster)) continue;
            e3.discard();
        }
        o.addProperty("detail", "debug_clear inventory + drops + hostiles");
        return o;
    }

    public static JsonObject actDebugPlace(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String blockId = req.has("block") ? req.get("block").getAsString() : "minecraft:oak_log";
        int bx = req.has("x") ? req.get("x").getAsInt() : (int)Math.floor(player.getX()) + 1;
        int by = req.has("y") ? req.get("y").getAsInt() : (int)Math.floor(player.getY()) - 1;
        int bz = req.has("z") ? req.get("z").getAsInt() : (int)Math.floor(player.getZ());
        Optional bHolder = BuiltInRegistries.BLOCK.get(Identifier.fromNamespaceAndPath((String)(blockId.contains(":") ? blockId.split(":")[0] : "minecraft"), (String)(blockId.contains(":") ? blockId.split(":")[1] : blockId)));
        if (bHolder.isEmpty()) {
            o.addProperty("detail", "debug_place unknown block: " + blockId);
            return o;
        }
        BlockState bs = ((Block)((net.minecraft.core.Holder.Reference)bHolder.get()).value()).defaultBlockState();
        level.setBlock(new BlockPos(bx, by, bz), bs, 3);
        BlockState after = level.getBlockState(new BlockPos(bx, by, bz));
        String afterId = BuiltInRegistries.BLOCK.getKey(after.getBlock()).toString();
        o.addProperty("detail", "debug_place " + blockId + " @ (" + bx + "," + by + "," + bz + ") -> actual=" + afterId + " air=" + after.isAir());
        return o;
    }

    public static JsonObject actDebugXp(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int levels = req.has("levels") ? req.get("levels").getAsInt() : 30;
        player.giveExperienceLevels(levels);
        o.addProperty("detail", "debug_xp +" + levels + " levels (now " + player.experienceLevel + ")");
        return o;
    }

    public static JsonObject actDebugFood(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int lvl = req.has("level") ? req.get("level").getAsInt() : 0;
        lvl = Math.max(0, Math.min(20, lvl));
        player.getFoodData().setFoodLevel(lvl);
        o.addProperty("detail", "debug_food level=" + lvl);
        return o;
    }

    public static JsonObject actDebugTime(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String which;
        String timeArg = which = req.has("value") ? req.get("value").getAsString().toLowerCase() : "night";
        if (which.equals("night")) {
            timeArg = "night";
        } else if (which.equals("day")) {
            timeArg = "day";
        } else if (which.equals("noon")) {
            timeArg = "noon";
        } else if (which.equals("midnight")) {
            timeArg = "midnight";
        }
        try {
            CraftAgentBridge.serverInstance.getCommands().performPrefixedCommand(CraftAgentBridge.serverInstance.createCommandSourceStack(), "time set " + timeArg);
            CraftAgentBridge.serverInstance.getCommands().performPrefixedCommand(CraftAgentBridge.serverInstance.createCommandSourceStack(), "gamerule doDaylightCycle false");
            o.addProperty("detail", "debug_time -> " + which + " (doDaylightCycle=false)");
            return o;
        }
        catch (Exception ex) {
            o.addProperty("detail", "debug_time failed: " + ex.getMessage());
        }
        return o;
    }

    public static JsonObject actDebugTeleportPlayer(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String name = req.has("name") ? req.get("name").getAsString() : "";
        double dist = req.has("dist") ? req.get("dist").getAsDouble() : 3.0;
        ServerPlayer target = null;
        for (ServerPlayer p : CraftAgentBridge.serverInstance.getPlayerList().getPlayers()) {
            if (!p.getName().getString().equalsIgnoreCase(name) || p == player) continue;
            target = p;
            break;
        }
        if (target == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "debug_teleport_player: player '" + name + "' not found (or is the bot)");
            return o;
        }
        ServerPlayer tgt = target;
        double lx = player.getLookAngle().x;
        double lz = player.getLookAngle().z;
        double len = Math.sqrt(lx * lx + lz * lz);
        if (len < 1.0E-6) {
            lx = 0.0;
            lz = 1.0;
            len = 1.0;
        }
        double tx = player.getX() + (lx /= len) * dist;
        double tz = player.getZ() + (lz /= len) * dist;
        int groundY = (int)player.getY();
        int y = 320;
        while ((double)y > player.getY() - 20.0) {
            BlockPos bp = BlockPos.containing((double)tx, (double)y, (double)tz);
            if (!level.getBlockState(bp).isAir() && level.getBlockState(bp.above()).isAir()) {
                groundY = y + 1;
                break;
            }
            --y;
        }
        double ty = (double)groundY + 0.0;
        tgt.teleportTo(tx, ty, tz);
        tgt.setYRot((float)Math.toDegrees(Math.atan2(-lx, lz)));
        tgt.setXRot(0.0f);
        System.out.println("[TP-DIAG] " + name + " -> (" + String.format("%.1f,%.1f,%.1f", tx, ty, tz) + ") botAt=(" + String.format("%.1f,%.1f,%.1f", player.getX(), player.getY(), player.getZ()) + ")");
        o.addProperty("detail", "debug_teleport_player " + name + " -> (" + String.format("%.1f,%.1f,%.1f", tx, ty, tz) + ") near bot");
        return o;
    }

    public static JsonObject actDebugSetFixture(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String fixture = req.has("fixture") ? req.get("fixture").getAsString().toLowerCase() : "platform";
        StringBuilder detail = new StringBuilder();

        // ── 1. Clear inventory + monsters + drops ──
        player.getInventory().clearContent();
        AABB global = AABB.ofSize(new Vec3(0.0, 64.0, 0.0), 100000.0, 100000.0, 100000.0);
        for (Entity e : level.getEntities(null, global)) {
            if (e instanceof ItemEntity || e instanceof Monster) e.discard();
        }
        detail.append("cleared ");

        // ── 2. Build 9×9 dirt platform at origin (y=63,64) ──
        BlockState dirt = Blocks.DIRT.defaultBlockState();
        for (int dx = -4; dx <= 4; dx++) {
            for (int dz = -4; dz <= 4; dz++) {
                level.setBlock(new BlockPos(dx, 63, dz), dirt, 3);
                level.setBlock(new BlockPos(dx, 64, dz), dirt, 3);
            }
        }
        detail.append("platform ");

        // ── 3. Cancel movement + stop flags + teleport bot to origin ──
        CraftAgentBridge.shouldStop = false;
        CraftAgentBridge.moveTarget = null;
        CraftAgentBridge.moveWaypoints = null;
        CraftAgentBridge.moveTicksLeft = 0;
        CraftAgentBridge.moveReached = false;
        player.teleportTo(level, 0.5, 65.0, 0.5, Set.of(), 0.0f, 0.0f, true);
        player.setYRot(0.0f);
        player.setXRot(0.0f);
        detail.append("bot@origin ");

        // ── 4. Give baseline oak_log x16 ──
        var oakItem = BuiltInRegistries.ITEM.get(Identifier.fromNamespaceAndPath("minecraft", "oak_log"));
        if (oakItem.isPresent()) {
            player.getInventory().add(new ItemStack((ItemLike)((Holder.Reference)oakItem.get()).value(), 16));
        }
        detail.append("+oak_log");

        // ── 5. Per-fixture setup ──
        // Helper: spawn entity at fixed offset
        java.util.function.BiConsumer<String, BlockPos> spawnEntity = (entityType, pos) -> {
            var eth = BuiltInRegistries.ENTITY_TYPE.get(Identifier.fromNamespaceAndPath("minecraft", entityType));
            if (eth.isEmpty()) return;
            EntityType<?> et = (EntityType<?>) ((Holder.Reference<?>) eth.get()).value();
            Entity e = et.spawn(level, null, pos, EntitySpawnReason.COMMAND, true, false);
            if (e == null) {
                e = et.create(level, new EntitySpawnRequest(EntitySpawnReason.COMMAND, true));
                if (e != null) { e.setPos(pos.getX(), pos.getY(), pos.getZ()); level.addFreshEntity(e); }
            }
            if (e instanceof Mob m) {
                m.finalizeSpawn(level, level.getCurrentDifficultyAt(pos), EntitySpawnReason.COMMAND, null);
                m.setPersistenceRequired();
                m.setNoAi(false);
                m.addEffect(new MobEffectInstance(MobEffects.FIRE_RESISTANCE, 999999, 0, false, false));
            }
        };

        // Helper: place block at offset
        java.util.function.BiConsumer<String, BlockPos> placeBlock = (blockId, pos) -> {
            var bHolder = BuiltInRegistries.BLOCK.get(Identifier.fromNamespaceAndPath("minecraft", blockId));
            if (bHolder.isPresent()) {
                level.setBlock(pos, ((Block) ((Holder.Reference<?>) bHolder.get()).value()).defaultBlockState(), 3);
            }
        };

        // Helper: give item
        java.util.function.BiConsumer<String, Integer> giveItem = (itemId, num) -> {
            var iHolder = BuiltInRegistries.ITEM.get(Identifier.fromNamespaceAndPath("minecraft", itemId));
            if (iHolder.isPresent()) {
                player.getInventory().add(new ItemStack((ItemLike)((Holder.Reference)iHolder.get()).value(), num));
            }
        };

        // Note: fixture is already lowercased above, so all case labels are lowercase.
        switch (fixture) {
            case "platform" -> {}
            case "attack", "combat", "searchforentity", "nearestentity" -> {
                CraftAgentBridge.serverInstance.getCommands().performPrefixedCommand(
                    CraftAgentBridge.serverInstance.createCommandSourceStack(), "time set night");
                CraftAgentBridge.serverInstance.getCommands().performPrefixedCommand(
                    CraftAgentBridge.serverInstance.createCommandSourceStack(), "gamerule doDaylightCycle false");
                spawnEntity.accept("zombie", new BlockPos(0, 66, 3));
                detail.append(" +zombie");
            }
            case "searchforblock" -> {
                placeBlock.accept("oak_log", new BlockPos(1, 64, 0));
                detail.append(" +oak_log_block");
            }
            case "collectitems", "collect_items" -> {
                var iHolder = BuiltInRegistries.ITEM.get(Identifier.fromNamespaceAndPath("minecraft", "oak_log"));
                if (iHolder.isPresent()) {
                    ItemStack stack = new ItemStack((ItemLike)((Holder.Reference)iHolder.get()).value(), 4);
                    level.addFreshEntity(new ItemEntity(level, 0.5, 65.0, 0.5, stack));
                    detail.append(" +dropped_item");
                }
            }
            case "collect" -> {
                // Build a 4-block-tall oak_log column for the collect/mining tool
                for (int h = 0; h < 4; h++) {
                    level.setBlock(new BlockPos(3, 64 + h, 0), Blocks.OAK_LOG.defaultBlockState(), 3);
                }
                detail.append(" +log_column");
            }
            case "build" -> {
                // Give materials for common blueprints (dirt_shelter needs ~30 dirt)
                giveItem.accept("dirt", 64);
                giveItem.accept("oak_log", 64);
                giveItem.accept("cobblestone", 64);
                detail.append(" +build_mats");
            }
            case "eat_item", "eatitem", "consume", "autosurvive" -> {
                player.getFoodData().setFoodLevel(5);
                giveItem.accept("apple", 4);
                detail.append(" +apple");
            }
            case "craft", "craftingplan", "equip", "equipitem", "discard", "discardsmart",
                 "move_slot", "moveslot", "move_to_hotbar", "selectslot", "select_slot",
                 "use_item", "useitem", "inspectgui", "inspect_gui", "closegui", "close_gui" -> {
                detail.append(" (baseline ok)");
            }
            case "place" -> {
                giveItem.accept("dirt", 16);
                detail.append(" +dirt");
            }
            case "clearfurnace", "smelt" -> {
                placeBlock.accept("furnace", new BlockPos(1, 64, 0));
                giveItem.accept("iron_ore", 4);
                detail.append(" +furnace+ore");
            }
            case "enchant" -> {
                player.giveExperienceLevels(30);
                giveItem.accept("diamond_sword", 1);
                detail.append(" +xp+sword");
            }
            case "chest", "transfer" -> {
                placeBlock.accept("chest", new BlockPos(1, 64, 0));
                BlockEntity be = level.getBlockEntity(new BlockPos(1, 64, 0));
                if (be instanceof MenuProvider mp) {
                    player.openMenu(mp);
                    detail.append(" +opened_chest_gui");
                } else {
                    detail.append(" +chest(no_gui)");
                }
            }
            case "activate_nearest_block" -> {
                placeBlock.accept("crafting_table", new BlockPos(1, 64, 0));
                detail.append(" +crafting_table");
            }
            case "useon", "use_on_entity" -> {
                spawnEntity.accept("cow", new BlockPos(1, 65, 2));
                detail.append(" +cow");
            }
            case "digdown" -> {
                for (int depth = 0; depth < 8; depth++) {
                    int dy = 63 - depth;
                    for (int ox = -1; ox <= 1; ox++)
                        for (int oz = -1; oz <= 1; oz++)
                            level.setBlock(new BlockPos(ox, dy, oz), dirt, 3);
                }
                detail.append(" +dig_pillar");
            }
            case "ride" -> {
                spawnEntity.accept("horse", new BlockPos(1, 65, 3));
                detail.append(" +horse");
            }
            case "fish" -> {
                giveItem.accept("fishing_rod", 1);
                detail.append(" +fishing_rod");
            }
            case "sleep", "gotobed" -> {
                CraftAgentBridge.serverInstance.getCommands().performPrefixedCommand(
                    CraftAgentBridge.serverInstance.createCommandSourceStack(), "time set night");
                CraftAgentBridge.serverInstance.getCommands().performPrefixedCommand(
                    CraftAgentBridge.serverInstance.createCommandSourceStack(), "gamerule doDaylightCycle false");
                placeBlock.accept("red_bed", new BlockPos(1, 64, 1));
                detail.append(" +bed");
            }
            case "villager_trades", "trade_with_villager" -> {
                var eth = BuiltInRegistries.ENTITY_TYPE.get(Identifier.fromNamespaceAndPath("minecraft", "villager"));
                if (eth.isPresent()) {
                    EntityType<?> et = (EntityType<?>) ((Holder.Reference<?>) eth.get()).value();
                    Entity e = et.spawn(level, null, new BlockPos(2, 65, 2), EntitySpawnReason.COMMAND, true, false);
                    if (e == null) {
                        e = et.create(level, new EntitySpawnRequest(EntitySpawnReason.COMMAND, true));
                        if (e != null) { e.setPos(2, 65, 2); level.addFreshEntity(e); }
                    }
                    if (e instanceof Mob m) {
                        m.finalizeSpawn(level, level.getCurrentDifficultyAt(new BlockPos(2, 65, 2)), EntitySpawnReason.COMMAND, null);
                        m.setPersistenceRequired(); m.setNoAi(false);
                    }
                    if (e instanceof Villager v) {
                        MerchantOffers offers = new MerchantOffers();
                        offers.add(new MerchantOffer(new ItemCost(Items.EMERALD, 1), new ItemStack(Items.BOOK, 1), 5, 5, 0.05f));
                        offers.add(new MerchantOffer(new ItemCost(Items.EMERALD, 1), new ItemStack(Items.WHEAT, 4), 5, 5, 0.05f));
                        v.setOffers(offers);
                        giveItem.accept("emerald", 16);
                        detail.append(" +villager_trades");
                    }
                }
            }
            case "build_portal" -> {
                giveItem.accept("obsidian", 14);
                giveItem.accept("flint_and_steel", 1);
                // Creative mode: infinite reach so portal builder can place top blocks
                CraftAgentBridge.serverInstance.getCommands().performPrefixedCommand(
                    CraftAgentBridge.serverInstance.createCommandSourceStack(),
                    "gamemode creative CraftAgent");
                detail.append(" +portal_mat+creative");
            }
            case "go_to_player", "attack_player" -> {
                for (ServerPlayer p : CraftAgentBridge.serverInstance.getPlayerList().getPlayers()) {
                    if (!p.getUUID().equals(player.getUUID())) {
                        p.teleportTo(level, 3.0, 65.0, 0.5, Set.of(), 0.0f, 0.0f, true);
                        detail.append(" +real_player_teleported");
                        break;
                    }
                }
            }
            default -> detail.append(" (no extra fixture)");
        }

        player.containerMenu.broadcastChanges();
        o.addProperty("detail", "fixture " + fixture + ": " + detail);
        return o;
    }

    public static JsonObject actDebugTeleportBot(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        double tz;
        double tx;
        ServerPlayer real = null;
        for (ServerPlayer p : CraftAgentBridge.serverInstance.getPlayerList().getPlayers()) {
            if (p == player) continue;
            real = p;
            break;
        }
        CraftAgentBridge.moveTarget = null;
        CraftAgentBridge.moveWaypoints = null;
        CraftAgentBridge.moveTicksLeft = 0;
        CraftAgentBridge.moveReached = false;
        if (req.has("x") && req.has("z")) {
            tx = req.get("x").getAsDouble();
            tz = req.get("z").getAsDouble();
        } else if (real != null) {
            tx = real.getX();
            tz = real.getZ() + 1.0;
        } else {
            tx = player.getX();
            tz = player.getZ();
        }
        int groundY = (int)player.getY();
        int y = 320;
        while ((double)y > player.getY() - 40.0) {
            BlockPos bp = BlockPos.containing((double)tx, (double)y, (double)tz);
            if (!level.getBlockState(bp).isAir() && level.getBlockState(bp.above()).isAir() && level.getBlockState(bp.above(2)).isAir()) {
                groundY = y + 1;
                break;
            }
            --y;
        }
        double ftx = tx;
        double fty = groundY;
        double ftz = tz;
        player.teleportTo(level, ftx, fty, ftz, Set.of(), 0.0f, 0.0f, true);
        player.setYRot(0.0f);
        player.setXRot(0.0f);
        o.addProperty("detail", "debug_teleport_bot -> (" + String.format("%.1f,%.1f,%.1f", tx, (double)groundY, tz) + ")");
        return o;
    }
}
