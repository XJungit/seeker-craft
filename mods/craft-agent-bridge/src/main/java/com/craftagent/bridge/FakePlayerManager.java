package com.craftagent.bridge;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.mojang.authlib.GameProfile;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Set;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.Holder;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.game.ClientboundEntityPositionSyncPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket;
import net.minecraft.network.protocol.game.ClientboundRotateHeadPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ClientInformation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.server.players.NameAndId;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.ItemLike;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.storage.LevelResource;

public class FakePlayerManager {

    public static boolean createFakePlayer() {
        MinecraftServer server = CraftAgentBridge.serverInstance;
        if (server == null) {
            return false;
        }
        if (CraftAgentBridge.fakePlayer != null) {
            return true;
        }
        if (CraftAgentBridge.fakePlayerSpawning) {
            return false;
        }
        CraftAgentBridge.fakePlayerSpawning = true;
        try {
            ServerLevel level = server.getLevel(Level.OVERWORLD);
            if (level == null) {
                level = ((ServerPlayer)server.getPlayerList().getPlayers().get(0)).level();
            }
            String username = "CraftAgent";
            GameProfile profile = UUIDUtil.createOfflineProfile(username);
            ClientInformation clientInfo = ClientInformation.createDefault();
            EntityPlayerMPFake fake = new EntityPlayerMPFake(server, level, profile, clientInfo);
            server.getPlayerList().placeNewPlayer(new FakeClientConnection(PacketFlow.SERVERBOUND), fake, new CommonListenerCookie(profile, 0, clientInfo, false));
            server.getPlayerList().op(new NameAndId(profile));
            int safeY = getSurfaceY(level, 0, 0);
            if (safeY < 1) safeY = 64;
            fake.teleportTo(level, 0.5, safeY, 0.5, Set.of(), 0.0f, 0.0f, true);
            fake.setHealth(20.0f);
            fake.unsetRemovedPublic();
            fake.getAttribute(Attributes.STEP_HEIGHT).setBaseValue(0.6f);
            fake.gameMode.changeGameModeForPlayer(GameType.SURVIVAL);
            server.getPlayerList().broadcastAll(new ClientboundRotateHeadPacket(fake, (byte)(fake.yHeadRot * 256.0f / 360.0f)), level.dimension());
            server.getPlayerList().broadcastAll(ClientboundEntityPositionSyncPacket.of(fake), level.dimension());
            server.getPlayerList().broadcastAll(new ClientboundPlayerInfoUpdatePacket(ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER, fake));
            CraftAgentBridge.fakePlayer = fake;
            loadFakePlayerData(fake);
            System.out.println("[craft-agent-bridge] Fake player created: " + username + " at (0.5, 64.0, 0.5)");
            return true;
        }
        catch (Exception e) {
            System.err.println("[craft-agent-bridge] Failed to create fake player: " + e.getMessage());
            e.printStackTrace();
            CraftAgentBridge.fakePlayer = null;
            return false;
        }
        finally {
            CraftAgentBridge.fakePlayerSpawning = false;
        }
    }

    public static void removeFakePlayer() {
        if (CraftAgentBridge.fakePlayer == null) {
            return;
        }
        try {
            CraftAgentBridge.fakePlayer.kill(CraftAgentBridge.fakePlayer.level());
        }
        catch (Exception e) {
            System.err.println("[craft-agent-bridge] Error removing fake player: " + e.getMessage());
        }
        CraftAgentBridge.fakePlayer = null;
    }

    static EntityPlayerMPFake getFakePlayer() {
        return CraftAgentBridge.fakePlayer;
    }

    public static ServerPlayer getFirstPlayer(MinecraftServer server) {
        if (CraftAgentBridge.fakePlayer != null && (CraftAgentBridge.fakePlayer.isDeadOrDying() || !CraftAgentBridge.fakePlayer.isAlive())) {
            System.out.println("[craft-agent-bridge] fakePlayer dead, reviving...");
            CraftAgentBridge.fakePlayer.setHealth(20.0f);
            CraftAgentBridge.fakePlayer.unsetRemovedPublic();
            int safeY = getSurfaceY(CraftAgentBridge.fakePlayer.level(), 0, 0);
            if (safeY < 1) safeY = 64;
            CraftAgentBridge.fakePlayer.teleportTo(CraftAgentBridge.fakePlayer.level(), 0.5, safeY, 0.5, Set.of(), 0.0f, 0.0f, true);
            ServerPlayer real = null;
            for (ServerPlayer p : server.getPlayerList().getPlayers()) {
                if (p == CraftAgentBridge.fakePlayer) continue;
                real = p;
                break;
            }
            if (real != null) {
                int gy = getSurfaceY(real.level(), (int)real.getX(), (int)real.getZ());
                if (gy < 1) gy = (int)real.getY();
                CraftAgentBridge.fakePlayer.teleportTo(real.level(), real.getX(), gy + 1.5, real.getZ(), Set.of(), 0.0f, 0.0f, true);
            }
            System.out.println("[craft-agent-bridge] revived at (" + CraftAgentBridge.fakePlayer.getX() + "," + CraftAgentBridge.fakePlayer.getY() + "," + CraftAgentBridge.fakePlayer.getZ() + ")");
        }
        if (CraftAgentBridge.fakePlayer != null) {
            return CraftAgentBridge.fakePlayer;
        }
        List players = server.getPlayerList().getPlayers();
        return players.isEmpty() ? null : (ServerPlayer)players.get(0);
    }

    private static int getSurfaceY(Level level, int x, int z) {
        for (int y = level.getMaxY() - 1; y > level.getMinY(); y--) {
            var bs = level.getBlockState(new net.minecraft.core.BlockPos(x, y, z));
            if (!bs.isAir() && !bs.canBeReplaced()
                && level.getBlockState(new net.minecraft.core.BlockPos(x, y + 1, z)).isAir()
                && level.getBlockState(new net.minecraft.core.BlockPos(x, y + 2, z)).isAir()) {
                return y + 1;
            }
        }
        return -1;
    }

    private static final String SAVE_FILE_NAME = "craftagent_inv.json";

    public static void saveFakePlayerData() {
        ServerPlayer p = CraftAgentBridge.fakePlayer;
        if (p == null) return;
        try {
            JsonObject data = new JsonObject();
            JsonArray invArr = new JsonArray();
            Inventory inv = p.getInventory();
            for (int i = 0; i < inv.getContainerSize(); i++) {
                ItemStack stack = inv.getItem(i);
                if (!stack.isEmpty()) {
                    JsonObject slotObj = new JsonObject();
                    slotObj.addProperty("slot", i);
                    slotObj.addProperty("id", BuiltInRegistries.ITEM.getKey(stack.getItem()).toString());
                    slotObj.addProperty("count", stack.getCount());
                    invArr.add(slotObj);
                }
            }
            data.add("inventory", invArr);
            data.addProperty("selected_slot", inv.getSelectedSlot());
            data.addProperty("health", p.getHealth());
            data.addProperty("food", p.getFoodData().getFoodLevel());
            data.addProperty("xp_level", p.experienceLevel);

            Path path = getSavePath();
            if (path != null) {
                Files.createDirectories(path.getParent());
                Files.writeString(path, data.toString());
                System.out.println("[craft-agent-bridge] Saved fake player data (" + invArr.size() + " items) to " + path);
            }
        } catch (Exception e) {
            System.err.println("[craft-agent-bridge] Failed to save fake player data: " + e.getMessage());
            e.printStackTrace();
        }
    }

    public static void loadFakePlayerData(ServerPlayer player) {
        try {
            Path path = getSavePath();
            if (path == null || !Files.exists(path)) return;

            String json = Files.readString(path);
            JsonObject data = new com.google.gson.Gson().fromJson(json, JsonObject.class);
            if (data == null) return;

            if (data.has("inventory")) {
                Inventory inv = player.getInventory();
                inv.clearContent();
                JsonArray invArr = data.getAsJsonArray("inventory");
                for (int i = 0; i < invArr.size(); i++) {
                    JsonObject slotObj = invArr.get(i).getAsJsonObject();
                    int slot = slotObj.get("slot").getAsInt();
                    String id = slotObj.get("id").getAsString();
                    int count = slotObj.get("count").getAsInt();

                    Identifier loc = id.contains(":") ? Identifier.tryParse(id) : Identifier.fromNamespaceAndPath("minecraft", id);
                    if (loc == null) continue;

                    var itemOpt = BuiltInRegistries.ITEM.get(loc);
                    if (itemOpt.isPresent()) {
                        ItemStack stack = new ItemStack((ItemLike)((Holder.Reference)itemOpt.get()).value(), count);
                        inv.setItem(slot, stack);
                    }
                }
                if (data.has("selected_slot")) {
                    inv.setSelectedSlot(data.get("selected_slot").getAsInt());
                }
            }

            if (data.has("health")) player.setHealth(data.get("health").getAsFloat());
            if (data.has("food")) player.getFoodData().setFoodLevel(data.get("food").getAsInt());
            if (data.has("xp_level")) player.experienceLevel = data.get("xp_level").getAsInt();

            System.out.println("[craft-agent-bridge] Restored fake player data from " + path);
        } catch (Exception e) {
            System.err.println("[craft-agent-bridge] Failed to load fake player data: " + e.getMessage());
            e.printStackTrace();
        }
    }

    private static Path getSavePath() {
        MinecraftServer server = CraftAgentBridge.serverInstance;
        if (server == null) return null;
        try {
            return server.getWorldPath(LevelResource.ROOT).resolve(SAVE_FILE_NAME);
        } catch (Exception e) {
            return null;
        }
    }

    static class EntityPlayerMPFake extends ServerPlayer {
        public EntityPlayerMPFake(MinecraftServer server, ServerLevel worldIn, GameProfile profile, ClientInformation cli) {
            super(server, worldIn, profile, cli);
        }

        public void tick() {
            if (this.level().getServer().getTickCount() % 10 == 0) {
                this.level().getChunkSource().move(this);
            }
            try {
                super.tick();
                this.doTick();
            }
            catch (NullPointerException nullPointerException) {
            }
        }

        public void unsetRemovedPublic() {
            super.unsetRemoved();
        }
    }
}
