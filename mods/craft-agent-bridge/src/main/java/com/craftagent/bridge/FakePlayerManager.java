package com.craftagent.bridge;

import com.mojang.authlib.GameProfile;
import java.util.List;
import java.util.Set;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.game.ClientboundEntityPositionSyncPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket;
import net.minecraft.network.protocol.game.ClientboundRotateHeadPacket;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ClientInformation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.server.players.NameAndId;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;

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
            fake.teleportTo(level, 0.5, 64.0, 0.5, Set.of(), 0.0f, 0.0f, true);
            fake.setHealth(20.0f);
            fake.unsetRemovedPublic();
            fake.getAttribute(Attributes.STEP_HEIGHT).setBaseValue(0.6f);
            fake.gameMode.changeGameModeForPlayer(GameType.SURVIVAL);
            server.getPlayerList().broadcastAll(new ClientboundRotateHeadPacket(fake, (byte)(fake.yHeadRot * 256.0f / 360.0f)), level.dimension());
            server.getPlayerList().broadcastAll(ClientboundEntityPositionSyncPacket.of(fake), level.dimension());
            server.getPlayerList().broadcastAll(new ClientboundPlayerInfoUpdatePacket(ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER, fake));
            CraftAgentBridge.fakePlayer = fake;
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
            removeFakePlayer();
            createFakePlayer();
            if (CraftAgentBridge.fakePlayer != null) {
                CraftAgentBridge.fakePlayer.setHealth(20.0f);
                ServerPlayer real = null;
                for (ServerPlayer p : server.getPlayerList().getPlayers()) {
                    if (p == CraftAgentBridge.fakePlayer) continue;
                    real = p;
                    break;
                }
                if (real != null) {
                    int gy = (int)real.getY();
                    CraftAgentBridge.fakePlayer.teleportTo(real.level(), real.getX(), gy + 1, real.getZ() + 1.0, Set.of(), 0.0f, 0.0f, true);
                }
            }
        }
        if (CraftAgentBridge.fakePlayer != null) {
            return CraftAgentBridge.fakePlayer;
        }
        List players = server.getPlayerList().getPlayers();
        return players.isEmpty() ? null : (ServerPlayer)players.get(0);
    }

    static class EntityPlayerMPFake extends ServerPlayer {
        public EntityPlayerMPFake(MinecraftServer server, ServerLevel worldIn, GameProfile profile, ClientInformation cli) {
            super(server, worldIn, profile, cli);
        }

        public void tick() {
            if (this.level().getServer().getTickCount() % 10 == 0) {
                this.connection.resetPosition();
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
