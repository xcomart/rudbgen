package comart.rudbgen.bridge.meta;

import com.google.gson.JsonObject;

import java.sql.Connection;
import java.sql.SQLException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;

/** Microsoft SQL Server. */
final class SqlServerVendor extends Vendor {

    @Override
    boolean names(String productName) {
        return productName.contains("microsoft sql server") || productName.contains("sql server");
    }

    @Override
    boolean hidesComments() {
        return true;
    }

    @Override
    Map<String, String> tableComments(Connection conn, JsonObject req) throws SQLException {
        if (Comments.elsewhere(conn, req)) {
            return Collections.emptyMap();
        }
        Comments.Filter schema = Comments.Filter.of(req, "schema", "schema_pattern");
        Comments.Filter table = Comments.Filter.of(req, "table", "table_pattern");
        StringBuilder sql = new StringBuilder(Comments.SQLSERVER_TABLES);
        List<String> args = new ArrayList<>();
        schema.append(sql, args, "s.name", Comments.ANSI_ESCAPE);
        table.append(sql, args, "o.name", Comments.ANSI_ESCAPE);
        return Comments.read(conn, sql.toString(), args, 2);
    }

    @Override
    Map<String, String> columnComments(Connection conn, JsonObject req) throws SQLException {
        if (Comments.elsewhere(conn, req)) {
            return Collections.emptyMap();
        }
        Comments.Filter schema = Comments.Filter.of(req, "schema", "schema_pattern");
        Comments.Filter table = Comments.Filter.of(req, "table", "table_pattern");
        Comments.Filter column = Comments.Filter.of(req, "column", "column_pattern");
        StringBuilder sql = new StringBuilder(Comments.SQLSERVER_COLUMNS);
        List<String> args = new ArrayList<>();
        schema.append(sql, args, "s.name", Comments.ANSI_ESCAPE);
        table.append(sql, args, "o.name", Comments.ANSI_ESCAPE);
        column.append(sql, args, "c.name", Comments.ANSI_ESCAPE);
        return Comments.read(conn, sql.toString(), args, 3);
    }
}
