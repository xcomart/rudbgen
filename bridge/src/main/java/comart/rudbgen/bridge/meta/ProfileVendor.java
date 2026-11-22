package comart.rudbgen.bridge.meta;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonPrimitive;
import comart.rudbgen.bridge.Json;

import java.sql.Connection;
import java.sql.DatabaseMetaData;
import java.sql.SQLException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

/**
 * A product a dialect profile describes, on top of the built-in its chain of
 * bases ends in.
 *
 * <p>A profile is the YAML file of {@code <config>/dialects} that rudbman and
 * rudbgen both read (rudbkit's {@code dialects}; the format is rudbman's
 * {@code docs/dialect-profiles.md}). {@code rudbgen-core} merges its
 * {@code bridge} half down the chain and sends it in {@code OPEN_SESSION} as
 * {@code profile}: {@code {id, base, bridge}}. Of that half this bridge reads
 * what it answers - {@code detect}, {@code catalog_is_schema} and
 * {@code comments} - and everything else is the base's. A key the profile
 * leaves out is the base's answer too; a {@code null} where the key takes one
 * is "none".
 *
 * <p>The comment queries are rudbman's: owner, table and, for columns, column,
 * then the comment, with {@code AND column = ?} or {@code LIKE ?} appended
 * for each filter the request has, on the column the profile's {@code filters}
 * names.
 */
public final class ProfileVendor extends Vendor {

    private final String id;
    private final Vendor base;
    private final JsonObject data;
    private final JsonObject comments;

    private ProfileVendor(String id, Vendor base, JsonObject data) {
        this.id = id;
        this.base = base;
        this.data = data;
        JsonObject c = Json.obj(data, "comments");
        this.comments = c == null ? new JsonObject() : c;
    }

    /**
     * The vendor a session reads with: its driver's profile, else the first
     * detecting profile whose words are in the server's product name, else the
     * built-in the product name names.
     *
     * @param profile the request's {@code profile}, may be {@code null}
     * @param detect  the request's {@code detect_profiles}, may be {@code null}
     * @param dbm     the connection's metadata, for its product name
     * @return the vendor, never {@code null}
     */
    public static Vendor forSession(JsonObject profile, JsonArray detect, DatabaseMetaData dbm) {
        if (profile != null) {
            return of(profile);
        }
        if (detect != null && !detect.isEmpty()) {
            String name;
            try {
                name = dbm.getDatabaseProductName();
            } catch (Exception | AbstractMethodError e) {
                name = null;
            }
            if (name != null) {
                String lower = name.toLowerCase(Locale.ROOT);
                for (JsonElement e : detect) {
                    if (e.isJsonObject()) {
                        ProfileVendor v = of(e.getAsJsonObject());
                        if (v.names(lower)) {
                            return v;
                        }
                    }
                }
            }
        }
        return Dialect.of(dbm).vendor();
    }

    /**
     * @param profile a session's profile, {@code {id, base, bridge}}
     * @return the vendor it describes
     */
    static ProfileVendor of(JsonObject profile) {
        JsonObject data = Json.obj(profile, "bridge");
        return new ProfileVendor(Json.str(profile, "id"),
                Dialect.byId(Json.str(profile, "base")).vendor(),
                data == null ? new JsonObject() : data);
    }

    /** @return the profile's id */
    public String id() {
        return id;
    }

    /** Whether any of the profile's {@code detect} words is in the name. */
    @Override
    boolean names(String productName) {
        JsonArray words = Json.arr(data, "detect");
        if (words == null || productName == null) {
            return false;
        }
        for (JsonElement w : words) {
            if (w.isJsonPrimitive()
                    && productName.contains(w.getAsString().toLowerCase(Locale.ROOT))) {
                return true;
            }
        }
        return false;
    }

    @Override
    public boolean catalogIsSchema() {
        Boolean value = flag(data, "catalog_is_schema");
        return value != null ? value : base.catalogIsSchema();
    }

    @Override
    boolean marksViewsInRemarks() {
        Boolean value = flag(comments, "marks_views_in_remarks");
        return value != null ? value : base.marksViewsInRemarks();
    }

    @Override
    boolean hidesComments() {
        Boolean value = flag(comments, "hides_comments");
        return value != null ? value : base.hidesComments();
    }

    @Override
    Map<String, String> tableComments(Connection conn, JsonObject req) throws SQLException {
        if (!comments.has("tables")) {
            return base.tableComments(conn, req);
        }
        return comments(conn, comments.get("tables"), req, 2, false);
    }

    @Override
    Map<String, String> columnComments(Connection conn, JsonObject req) throws SQLException {
        if (!comments.has("columns")) {
            return base.columnComments(conn, req);
        }
        return comments(conn, comments.get("columns"), req, 3, true);
    }

    /**
     * Runs a profile's comment query, keyed by its first {@code keys}
     * columns; {@code null} for none.
     */
    private Map<String, String> comments(Connection conn, JsonElement query, JsonObject req,
                                         int keys, boolean columns) throws SQLException {
        if (query == null || !query.isJsonObject()) {
            return Collections.emptyMap();
        }
        JsonObject q = query.getAsJsonObject();
        JsonObject filters = Json.obj(q, "filters");
        if (filters == null) {
            filters = new JsonObject();
        }
        String escape = "none".equals(Json.str(comments, "escape")) ? Comments.NO_ESCAPE
                : Comments.ANSI_ESCAPE;
        StringBuilder sql = new StringBuilder(Json.str(q, "sql"));
        List<String> args = new ArrayList<>();
        Set<String> used = new HashSet<>();
        String catalog = Json.str(req, "catalog");
        String catalogColumn = Json.str(filters, "catalog");
        if (catalogColumn != null && catalog != null && !catalog.isEmpty()) {
            sql.append(" AND ").append(catalogColumn).append(" = ?");
            args.add(catalog);
            used.add(catalogColumn);
        }
        pattern(sql, args, used, Json.str(filters, "schema"),
                Comments.Filter.of(req, "schema", "schema_pattern"), escape);
        pattern(sql, args, used, Json.str(filters, "table"),
                Comments.Filter.of(req, "table", "table_pattern"), escape);
        if (columns) {
            pattern(sql, args, used, Json.str(filters, "column"),
                    Comments.Filter.of(req, "column", "column_pattern"), escape);
        }
        return Comments.read(conn, sql.toString(), args, keys);
    }

    /**
     * Appends one filter on its column; a column two filters name is filtered
     * once, by the first with a value - the MySQL family's database answers
     * both as catalog and as schema.
     */
    private static void pattern(StringBuilder sql, List<String> args, Set<String> used,
                                String column, Comments.Filter filter, String escape) {
        if (column == null || filter.absent() || !used.add(column)) {
            return;
        }
        filter.append(sql, args, column, escape);
    }

    /** @return the key's truth value, {@code null} where it is left out */
    private static Boolean flag(JsonObject o, String key) {
        JsonElement e = o.get(key);
        if (e == null || e.isJsonNull()) {
            return null;
        }
        if (e instanceof JsonPrimitive p && p.isBoolean()) {
            return p.getAsBoolean();
        }
        return null;
    }
}
