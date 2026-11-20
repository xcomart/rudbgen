package comart.rudbgen.bridge.meta;

/**
 * MariaDB: MySQL's answers, read from {@link MySqlVendor} rather than copied,
 * except where its catalog differs.
 */
final class MariaDbVendor extends MySqlVendor {

    /**
     * Tested before MySQL: the MariaDB server reports "MariaDB", but several
     * MySQL-compatible builds put both words in the string.
     */
    @Override
    boolean names(String productName) {
        return productName.contains("mariadb");
    }
}
