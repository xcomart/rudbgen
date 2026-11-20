package comart.rudbgen.bridge.meta;

import com.google.gson.JsonObject;

import java.sql.Connection;
import java.sql.DatabaseMetaData;
import java.sql.SQLException;
import java.util.Collections;
import java.util.Map;

/**
 * Everything the bridge does differently per database product.
 *
 * <p>Each product the bridge knows is one subclass in this package, and its
 * {@link Dialect} constant holds an instance. The code that reads catalogs
 * asks {@code dialect.vendor()} what to do rather than testing which product
 * it is, so a product's habits are read in one file. rudbman's bridge has the
 * same class with more to answer; the two keep the same shape so that a
 * product added to one is added to the other the same way.
 *
 * <p>Every method's default is what the bridge does for {@link Dialect#OTHER}:
 * the portable, standard-JDBC path. That is safe by the rule every caller
 * already keeps - a product the bridge does not recognize must stay useful -
 * so a new product starts out as {@code OTHER} and overrides only where it
 * differs. Long product-specific SQL stays beside the code that runs it,
 * package-private, and the product's override is what picks it.
 *
 * <p>Adding a product:
 * <ol>
 *   <li>Write a subclass, starting from the closest existing product, and
 *       override what differs. {@link #names} is the one method every product
 *       answers.</li>
 *   <li>Give it a {@link Dialect} constant and a place in that enum's
 *       detection order.</li>
 *   <li>Run the container suites against a server of that product.</li>
 * </ol>
 */
public abstract class Vendor {

    Vendor() {
    }

    /**
     * @param productName {@link DatabaseMetaData#getDatabaseProductName()},
     *                    lower-cased
     * @return whether it names this product
     */
    abstract boolean names(String productName);

    // ------------------------------------------------------------------ names

    /**
     * Whether the JDBC catalog is the namespace a table sits in, the level
     * {@code INFORMATION_SCHEMA} calls a schema.
     *
     * <p>MySQL and MariaDB, which put the database in {@code TABLE_CAT} and
     * leave {@code TABLE_SCHEM} null.
     *
     * @return {@code false} by default
     */
    public boolean catalogIsSchema() {
        return false;
    }

    // --------------------------------------------------------------- comments

    /**
     * Whether {@code getTables} reports a view's type in its {@code REMARKS},
     * where no comment was written.
     *
     * @return {@code false} by default
     */
    boolean marksViewsInRemarks() {
        return false;
    }

    /**
     * Whether the driver leaves out comments the server holds, so that they
     * are read from the catalog.
     *
     * @return {@code false} by default
     */
    boolean hidesComments() {
        return false;
    }

    /**
     * @param conn the live connection
     * @param req  the request, for the filters to scope the query with
     * @return table comments, keyed by owner and table name; none by default
     * @throws SQLException if the query fails
     */
    Map<String, String> tableComments(Connection conn, JsonObject req) throws SQLException {
        return Collections.emptyMap();
    }

    /**
     * @param conn the live connection
     * @param req  the request, for the filters to scope the query with
     * @return column comments, keyed by owner, table name and column name;
     *         none by default
     * @throws SQLException if the query fails
     */
    Map<String, String> columnComments(Connection conn, JsonObject req) throws SQLException {
        return Collections.emptyMap();
    }
}
