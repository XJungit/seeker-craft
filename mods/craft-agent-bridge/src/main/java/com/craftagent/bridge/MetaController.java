package com.craftagent.bridge;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.net.HttpURLConnection;
import java.net.URL;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.util.Set;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;

public class MetaController {

    static JsonObject actClearChat(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        o.addProperty("detail", "clear_chat: mod side ack, Rust side should clear history");
        return o;
    }

    static JsonObject actListPlayers(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        JsonArray players = new JsonArray();
        for (ServerPlayer p : CraftAgentBridge.serverInstance.getPlayerList().getPlayers()) {
            JsonObject po = new JsonObject();
            po.addProperty("name", p.getName().getString());
            po.addProperty("uuid", p.getUUID().toString());
            po.add("position", (JsonElement)CraftAgentBridge.arr(p.getX(), p.getY(), p.getZ()));
            po.addProperty("dist", (Number)Math.sqrt(Math.pow(p.getX() - player.getX(), 2.0) + Math.pow(p.getY() - player.getY(), 2.0) + Math.pow(p.getZ() - player.getZ(), 2.0)));
            players.add((JsonElement)po);
        }
        o.add("players", (JsonElement)players);
        o.addProperty("count", (Number)players.size());
        o.addProperty("detail", "list_players: " + players.size() + " online");
        return o;
    }

    static JsonObject actStop(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        CraftAgentBridge.shouldStop = true;
        CraftAgentBridge.moveTarget = null;
        o.addProperty("detail", "stop: all actions cancelled");
        return o;
    }

    static JsonObject actSetGoal(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String goal;
        String string = goal = req.has("goal") ? req.get("goal").getAsString() : "";
        if (goal.isEmpty()) {
            CraftAgentBridge.currentGoal = null;
            o.addProperty("detail", "set_goal: cleared");
            return o;
        }
        CraftAgentBridge.currentGoal = goal;
        o.addProperty("detail", "set_goal: " + goal);
        return o;
    }

    static JsonObject actGetGoal(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        o.addProperty("goal", CraftAgentBridge.currentGoal != null ? CraftAgentBridge.currentGoal : "(none)");
        o.addProperty("detail", "get_goal: " + (CraftAgentBridge.currentGoal != null ? CraftAgentBridge.currentGoal : "none"));
        return o;
    }

    static JsonObject actSearchWiki(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String query = req.has("query") ? req.get("query").getAsString() : "";
        try {
            URL url = new URL("https://minecraft.wiki/w/" + URLEncoder.encode(query.replace(" ", "_"), "UTF-8"));
            HttpURLConnection conn = (HttpURLConnection)url.openConnection();
            conn.setRequestProperty("User-Agent", "Craft-Agent/1.0");
            conn.setConnectTimeout(5000);
            conn.setReadTimeout(10000);
            if (conn.getResponseCode() == 404) {
                o.addProperty("detail", "search_wiki: '" + query + "' not found on minecraft.wiki");
                return o;
            }
            try (BufferedReader wr = new BufferedReader(new InputStreamReader(conn.getInputStream(), StandardCharsets.UTF_8));){
                String line;
                StringBuilder sb = new StringBuilder();
                while ((line = wr.readLine()) != null) {
                    sb.append(line).append("\n");
                }
                String html = sb.toString();
                Object text = html.replaceAll("<script[^>]*>[\\s\\S]*?</script>", "").replaceAll("<style[^>]*>[\\s\\S]*?</style>", "").replaceAll("<[^>]+>", " ").replaceAll("&amp;", "&").replaceAll("&lt;", "<").replaceAll("&gt;", ">").replaceAll("&quot;", "\"").replaceAll("&#39;", "'").replaceAll("\\s+", " ").trim();
                if (((String)text).length() > 2000) {
                    text = ((String)text).substring(0, 2000) + "... [truncated]";
                }
                o.addProperty("content", (String)text);
                o.addProperty("detail", "search_wiki: " + query + " (" + ((String)text).length() + " chars)");
                return o;
            }
        }
        catch (Exception e5) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "search_wiki error: " + e5.getMessage());
        }
        return o;
    }

    static JsonObject actTeleportTo(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        ServerLevel targetLevel;
        String dimension = req.has("dimension") ? req.get("dimension").getAsString() : "the_nether";
        switch (dimension.toLowerCase()) {
            case "the_nether":
            case "nether": {
                targetLevel = CraftAgentBridge.serverInstance.getLevel(Level.NETHER);
                break;
            }
            case "the_end":
            case "end": {
                targetLevel = CraftAgentBridge.serverInstance.getLevel(Level.END);
                break;
            }
            default: {
                targetLevel = CraftAgentBridge.serverInstance.getLevel(Level.OVERWORLD);
            }
        }
        if (targetLevel == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "teleport_to: dimension '" + dimension + "' not available");
            return o;
        }
        double scale = 1.0;
        if (level.dimension() == Level.NETHER && targetLevel.dimension() == Level.OVERWORLD) {
            scale = 8.0;
        } else if (level.dimension() == Level.OVERWORLD && targetLevel.dimension() == Level.NETHER) {
            scale = 0.125;
        }
        double tx = player.getX() * scale;
        double tz = player.getZ() * scale;
        double ty = player.getY();
        if (targetLevel.dimension() == Level.NETHER) {
            ty = Math.min(ty, 120.0);
        } else if (targetLevel.dimension() == Level.END) {
            tx = 0.0;
            ty = 65.0;
            tz = 0.0;
        }
        player.teleportTo(targetLevel, tx, ty, tz, Set.of(), player.getYRot(), player.getXRot(), false);
        player.containerMenu.broadcastChanges();
        o.addProperty("detail", "teleport_to " + dimension + " at (" + String.format("%.1f", tx) + "," + String.format("%.1f", ty) + "," + String.format("%.1f", tz) + ")");
        return o;
    }

    static JsonObject actNavTo(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        double tx = req.get("x").getAsDouble();
        double ty = req.get("y").getAsDouble();
        double tz = req.get("z").getAsDouble();
        com.craftagent.bridge.pathing.PlayerNavManager.get().navigateTo(tx, ty, tz);
        o.addProperty("detail", "nav_to (" + String.format("%.1f", tx) + "," + String.format("%.1f", ty) + "," + String.format("%.1f", tz) + ") started");
        return o;
    }

    static JsonObject actNavStatus(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        o.addProperty("detail", com.craftagent.bridge.pathing.PlayerNavManager.get().statusString());
        return o;
    }

    static JsonObject actNavStop(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        com.craftagent.bridge.pathing.PlayerNavManager.get().stop();
        o.addProperty("status", "ok");
        o.addProperty("detail", "nav stopped");
        return o;
    }

    static JsonObject actGoalExecute(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        String goalType = req.get("goal_type").getAsString();
        String param = req.has("param") ? req.get("param").getAsString() : "";
        int count = req.has("count") ? req.get("count").getAsInt() : 1;
        GoalEngine.get().start(goalType, param, count);
        o.addProperty("status", "ok");
        o.addProperty("detail", "goal started: " + goalType + " " + param + " x" + count);
        return o;
    }

    static JsonObject actGoalStatus(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        o.addProperty("detail", GoalEngine.get().statusString());
        // #4 可观测性：把目标栈作为结构化 progress 返回，LLM 可感知内部进度
        var stack = GoalEngine.get().progressStack();
        var arr = new com.google.gson.JsonArray();
        for (var s : stack) arr.add(s);
        o.add("progress", arr);
        o.addProperty("state", GoalEngine.get().status().name());
        return o;
    }
}
