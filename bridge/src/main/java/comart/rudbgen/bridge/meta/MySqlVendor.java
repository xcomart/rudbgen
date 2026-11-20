package comart.rudbgen.bridge.meta;

import com.google.gson.JsonObject;

import java.sql.Connection;
import java.sql.SQLException;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/**
 * MySQL. {@link MariaDbVendor} extends it and overrides where the fork parts
 * company.
 */
class MySqlVendor extends Vendor {

    @Override
    boolean names(String productName) {
        return productName.contains("mysql");
    }

    @Override
    public boolean catalogIsSchema() {
        return true;
    }

    /** Connector/J puts {@code VIEW} in a view's {@code REMARKS}. */
    @Override
    boolean marksViewsInRemarks() {
        return true;
    }

    @Override
    boolean hidesComments() {
        return true;
    }

    @Override
    Map<String, String> tableComments(Connection conn, JsonObject req) throws SQLException {
        Comments.Filter table = Comments.Filter.of(req, "table", "table_pattern");
        StringBuilder sql = new StringBuilder(Comments.MYSQL_TABLES);
        List<String> args = new ArrayList<>();
        Comments.mysqlDatabase(sql, args, req);
        table.append(sql, args, "TABLE_NAME", Comments.NO_ESCAPE);
        return Comments.read(conn, sql.toString(), args, 2);
    }

    @Override
    Map<String, String> columnComments(Connection conn, JsonObject req) throws SQLException {
        Comments.Filter table = Comments.Filter.of(req, "table", "table_pattern");
        Comments.Filter column = Comments.Filter.of(req, "column", "column_pattern");
        StringBuilder sql = new StringBuilder(Comments.MYSQL_COLUMNS);
        List<String> args = new ArrayList<>();
        Comments.mysqlDatabase(sql, args, req);
        table.append(sql, args, "TABLE_NAME", Comments.NO_ESCAPE);
        column.append(sql, args, "COLUMN_NAME", Comments.NO_ESCAPE);
        return Comments.read(conn, sql.toString(), args, 3);
    }
}
