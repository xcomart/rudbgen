package comart.rudbgen.bridge.meta;

/** SQLite. */
final class SqliteVendor extends Vendor {

    @Override
    boolean names(String productName) {
        return productName.contains("sqlite");
    }
}
