package comart.rudbgen.bridge.meta;

/** PostgreSQL and wire-compatible forks that keep the product name. */
final class PostgresVendor extends Vendor {

    @Override
    boolean names(String productName) {
        return productName.contains("postgresql") || productName.contains("postgres");
    }
}
