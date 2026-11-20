package comart.rudbgen.bridge.meta;

import com.google.gson.JsonObject;

import java.sql.Connection;
import java.sql.SQLException;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/** Oracle Database. */
final class OracleVendor extends Vendor {

    @Override
    boolean names(String productName) {
        return productName.contains("oracle");
    }

    @Override
    boolean hidesComments() {
        return true;
    }

    @Override
    Map<String, String> tableComments(Connection conn, JsonObject req) throws SQLException {
        Comments.Filter schema = Comments.Filter.of(req, "schema", "schema_pattern");
        Comments.Filter table = Comments.Filter.of(req, "table", "table_pattern");
        StringBuilder sql = new StringBuilder(Comments.ORACLE_TABLES);
        List<String> args = new ArrayList<>();
        Comments.oracleOwner(sql, args, schema);
        table.append(sql, args, "TABLE_NAME", Comments.ANSI_ESCAPE);
        return Comments.read(conn, sql.toString(), args, 2);
    }

    @Override
    Map<String, String> columnComments(Connection conn, JsonObject req) throws SQLException {
        Comments.Filter schema = Comments.Filter.of(req, "schema", "schema_pattern");
        Comments.Filter table = Comments.Filter.of(req, "table", "table_pattern");
        Comments.Filter column = Comments.Filter.of(req, "column", "column_pattern");
        StringBuilder sql = new StringBuilder(Comments.ORACLE_COLUMNS);
        List<String> args = new ArrayList<>();
        Comments.oracleOwner(sql, args, schema);
        table.append(sql, args, "TABLE_NAME", Comments.ANSI_ESCAPE);
        column.append(sql, args, "COLUMN_NAME", Comments.ANSI_ESCAPE);
        return Comments.read(conn, sql.toString(), args, 3);
    }
}
