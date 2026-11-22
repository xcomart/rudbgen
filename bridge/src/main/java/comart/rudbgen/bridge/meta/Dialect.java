package comart.rudbgen.bridge.meta;

import java.sql.DatabaseMetaData;
import java.util.Locale;

/**
 * The database product behind a session, to the resolution the bridge actually
 * needs.
 *
 * <p>Table and column comments cannot be answered through JDBC alone: several
 * drivers report none where the server holds one, and a few report a
 * placeholder where the server holds none ({@link Comments}). Reconciling that
 * needs a vendor catalog query, and a vendor catalog query needs to know the
 * vendor.
 *
 * <p>The discriminator is {@link DatabaseMetaData#getDatabaseProductName()},
 * the same string {@code SESSION_INFO} reports, matched case-insensitively on a
 * substring. Product names are marketing text and do drift between versions, so
 * an unrecognized name is not an error: it maps to {@link #OTHER}, which means
 * "portable paths only". Every caller must stay useful under {@link #OTHER}.
 */
public enum Dialect {

    /** H2, product name {@code "H2"}. */
    H2(new H2Vendor()),
    /** PostgreSQL and wire-compatible forks that keep the product name. */
    POSTGRESQL(new PostgresVendor()),
    /** MySQL. */
    MYSQL(new MySqlVendor()),
    /** MariaDB. Reported separately from MySQL because its catalog differs. */
    MARIADB(new MariaDbVendor()),
    /** Oracle Database. */
    ORACLE(new OracleVendor()),
    /** Microsoft SQL Server. */
    SQLSERVER(new SqlServerVendor()),
    /** SQLite. */
    SQLITE(new SqliteVendor()),
    /** IBM Db2. */
    DB2(new Db2Vendor()),
    /** Anything unrecognized. Only portable, standard-JDBC paths may be used. */
    OTHER(new PortableVendor());

    /**
     * The order products are tried in. MariaDB must be tested before MySQL:
     * the MariaDB server reports "MariaDB" but several MySQL-compatible builds
     * put both words in the string, and the sequence support hangs on which
     * one is really there.
     */
    private static final Dialect[] DETECTION = {
        MARIADB, MYSQL, POSTGRESQL, ORACLE, SQLSERVER, SQLITE, H2, DB2,
    };

    private final Vendor vendor;

    Dialect(Vendor vendor) {
        this.vendor = vendor;
    }

    /** @return everything the bridge does differently for this product */
    public Vendor vendor() {
        return vendor;
    }

    /**
     * The built-in a dialect profile's chain of bases ends in, as rudbman's SQL
     * layer and rudbkit name it ({@code base} of a session's profile).
     *
     * @param id the built-in's id, such as {@code mysql}; may be {@code null}
     * @return its product, or {@link #OTHER} for {@code generic} and anything
     *         this bridge has no vendor for
     */
    public static Dialect byId(String id) {
        if (id == null) {
            return OTHER;
        }
        switch (id.toLowerCase(Locale.ROOT)) {
            case "h2":       return H2;
            case "postgres": return POSTGRESQL;
            case "mysql":    return MYSQL;
            case "mariadb":  return MARIADB;
            case "oracle":   return ORACLE;
            case "mssql":    return SQLSERVER;
            case "sqlite":   return SQLITE;
            case "db2":      return DB2;
            default:         return OTHER;
        }
    }

    /**
     * Identifies the product behind a connection.
     *
     * <p>Never throws: a driver that fails to answer what it is still deserves
     * the portable paths rather than a failed request.
     *
     * @param dbm the connection metadata
     * @return the matching dialect, or {@link #OTHER}
     */
    public static Dialect of(DatabaseMetaData dbm) {
        String name;
        try {
            name = dbm.getDatabaseProductName();
        } catch (Exception | AbstractMethodError e) {
            return OTHER;
        }
        return byProductName(name);
    }

    /**
     * Identifies the product from its name.
     *
     * @param productName the value of {@code getDatabaseProductName()}, may be
     *                    {@code null}
     * @return the matching dialect, or {@link #OTHER}
     */
    public static Dialect byProductName(String productName) {
        if (productName == null) {
            return OTHER;
        }
        String n = productName.toLowerCase(Locale.ROOT);
        for (Dialect d : DETECTION) {
            if (d.vendor.names(n)) {
                return d;
            }
        }
        return OTHER;
    }
}
