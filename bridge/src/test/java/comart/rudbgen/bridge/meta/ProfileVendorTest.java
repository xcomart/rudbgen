package comart.rudbgen.bridge.meta;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import comart.rudbgen.bridge.Ops;
import comart.rudbgen.bridge.support.H2;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.HashMap;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * A session opened with a dialect profile reads comments the profile's way,
 * on an H2 whose own driver reports none of the ones the queries answer.
 */
class ProfileVendorTest {

    /**
     * Tables and columns of {@code APP}, each "from the profile": what the
     * driver's {@code REMARKS} lacks, the profile's queries fill in.
     */
    private static final String PROFILE = "{"
            + "\"id\": \"h2ish\", \"base\": \"generic\", \"bridge\": {"
            + "  \"detect\": [\"h2\"],"
            + "  \"comments\": {"
            + "    \"hides_comments\": true,"
            + "    \"tables\": {\"sql\": \"SELECT TABLE_SCHEMA, TABLE_NAME, 'table from the profile'"
            + " FROM INFORMATION_SCHEMA.TABLES WHERE 1 = 1\","
            + "      \"filters\": {\"schema\": \"TABLE_SCHEMA\", \"table\": \"TABLE_NAME\"}},"
            + "    \"columns\": {\"sql\": \"SELECT TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME,"
            + " 'column from the profile' FROM INFORMATION_SCHEMA.COLUMNS WHERE 1 = 1\","
            + "      \"filters\": {\"schema\": \"TABLE_SCHEMA\", \"table\": \"TABLE_NAME\","
            + "        \"column\": \"COLUMN_NAME\"}}"
            + "  }"
            + "}}";

    private String url;
    private long setup;

    @BeforeEach
    void setUp() {
        url = H2.freshUrl();
        setup = H2.open(url);
        H2.exec(setup, "create schema app");
        H2.exec(setup, "create table app.t (id int primary key, note varchar(10))");
        H2.exec(setup, "comment on column app.t.id is 'the key'");
    }

    @AfterEach
    void tearDown() {
        H2.close(setup);
    }

    private static JsonObject profile() {
        return JsonParser.parseString(PROFILE).getAsJsonObject();
    }

    private static Map<String, String> remarks(long session, String kind) {
        JsonObject req = new JsonObject();
        req.addProperty("kind", kind);
        req.addProperty("schema", "APP");
        req.addProperty("table", "T");
        JsonArray items = H2.call(Ops.DESCRIBE, session, 0, req).json().getAsJsonArray("items");
        Map<String, String> out = new HashMap<>();
        for (JsonElement e : items) {
            JsonObject o = e.getAsJsonObject();
            JsonElement r = o.get("remarks");
            out.put(o.get("name").getAsString(), r == null || r.isJsonNull() ? null : r.getAsString());
        }
        return out;
    }

    @Test
    void theDriversProfileFillsTheCommentsTheDriverLacks() {
        JsonObject extra = new JsonObject();
        extra.add("profile", profile());
        long session = H2.open(url, extra);
        try {
            assertEquals("table from the profile", remarks(session, "tables").get("T"));
            Map<String, String> columns = remarks(session, "columns");
            assertEquals("the key", columns.get("ID"), "the driver's own comment stands");
            assertEquals("column from the profile", columns.get("NOTE"));
        } finally {
            H2.close(session);
        }
    }

    @Test
    void aDetectingProfileIsFoundByTheProductName() {
        JsonArray detect = new JsonArray();
        detect.add(profile());
        JsonObject extra = new JsonObject();
        extra.add("detect_profiles", detect);
        long session = H2.open(url, extra);
        try {
            assertEquals("table from the profile", remarks(session, "tables").get("T"));
        } finally {
            H2.close(session);
        }
    }

    @Test
    void withoutAProfileTheProductsOwnVendorReads() {
        long session = H2.open(url);
        try {
            assertNull(remarks(session, "tables").get("T"), "H2 itself has none to give");
        } finally {
            H2.close(session);
        }
    }

    @Test
    void aKeyLeftOutIsTheBasesAndANullOneIsNone() {
        JsonObject bare = JsonParser.parseString(
                "{\"id\": \"b\", \"base\": \"mysql\", \"bridge\": {\"comments\": {\"tables\": null}}}")
                .getAsJsonObject();
        ProfileVendor vendor = ProfileVendor.of(bare);
        assertTrue(vendor.catalogIsSchema(), "MySQL's");
        assertTrue(vendor.hidesComments(), "MySQL's");
        assertTrue(vendor.marksViewsInRemarks(), "MySQL's");
        assertEquals("b", vendor.id());

        JsonObject generic = JsonParser.parseString(
                "{\"id\": \"g\", \"base\": \"generic\", \"bridge\": {\"catalog_is_schema\": true}}")
                .getAsJsonObject();
        ProfileVendor g = ProfileVendor.of(generic);
        assertTrue(g.catalogIsSchema(), "its own");
        assertFalse(g.hidesComments(), "generic's");
        assertFalse(g.names("h2"), "no detect, no match");
    }

    @Test
    void everyBaseIdNamesItsProduct() {
        assertEquals(Dialect.MYSQL, Dialect.byId("mysql"));
        assertEquals(Dialect.POSTGRESQL, Dialect.byId("postgres"));
        assertEquals(Dialect.SQLSERVER, Dialect.byId("mssql"));
        assertEquals(Dialect.OTHER, Dialect.byId("generic"));
        assertEquals(Dialect.OTHER, Dialect.byId(null));
    }
}
