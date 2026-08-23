package comart.rudbgen.bridge.codec;

import java.sql.Types;

/**
 * Physical encodings of the {@code RDB1} batch codec (README.md, "RDB1 batch codec").
 *
 * <p>The kind is a <em>transport</em> decision only. Presentation (right
 * alignment, NULL rendering, copy formatting) is driven by the logical JDBC type
 * carried in the {@code columns[]} array of the {@code EXECUTE} response.
 *
 * <p>The kind of a given column may differ between batches of the same cursor:
 * a batch in which every value of a column is NULL is emitted as {@link #NULLS}
 * regardless of the column's declared type. Decoders must therefore switch on
 * the kind byte per batch, not once per cursor.
 */
public final class ColumnKind {

    /** Every row is NULL; no value area follows the validity bitmap. */
    public static final int NULLS = 0;
    /** {@code row_count} 64-bit signed integers. */
    public static final int I64 = 1;
    /** {@code row_count} IEEE-754 doubles. */
    public static final int F64 = 2;
    /** Packed bits, LSB-first, {@code ceil(row_count/8)} bytes. */
    public static final int BOOL = 3;
    /** {@code u32 offsets[row_count+1]} followed by UTF-8 bytes. */
    public static final int STR = 4;
    /** Same layout as {@link #STR}, raw bytes. */
    public static final int BIN = 5;
    // Kind 6 was rudbman's LOB reference column, {@code (u64 lob_id, u64 size)}
    // per row, resolved by a follow-up LOB_READ. Both are retired here
    // (architecture.md D3); the number stays spent so a batch never means one
    // thing in rudbman and another here.

    private ColumnKind() {
    }

    /**
     * Chooses the physical encoding for a result column.
     *
     * <p>Everything that is not a plain integer, float, boolean or byte array
     * becomes {@link #STR}: DECIMAL keeps full precision as text, and date /
     * time / timestamp keep whatever the driver formatted, because the driver's
     * text is the only authority on the time zone that was applied.
     *
     * @param sqlType   a {@link java.sql.Types} constant
     * @param precision the column precision, used to tell a real boolean
     *                  {@code BIT} from a MySQL-style {@code BIT(n)} bit string
     * @return one of the {@code kind} constants of this class
     */
    public static int forSqlType(int sqlType, int precision) {
        switch (sqlType) {
            case Types.TINYINT:
            case Types.SMALLINT:
            case Types.INTEGER:
            case Types.BIGINT:
                return I64;
            case Types.REAL:
            case Types.FLOAT:
            case Types.DOUBLE:
                return F64;
            case Types.BOOLEAN:
                return BOOL;
            case Types.BIT:
                // MySQL reports BIT(n>1) as Types.BIT but hands back a byte
                // string, not a boolean. Only a single bit is a boolean.
                return precision <= 1 ? BOOL : BIN;
            case Types.BINARY:
            case Types.VARBINARY:
            case Types.LONGVARBINARY:
                return BIN;
            case Types.BLOB:
                // rudbman handed back a reference and let the UI pull the bytes
                // on demand. rudbgen has no LOB_READ to pull with, and the only
                // statements it runs are the four custom catalogue queries
                // (architecture.md D9), whose values are names and comments. So
                // a LOB is materialised like any other value of its shape.
                return BIN;
            case Types.CLOB:
            case Types.NCLOB:
                // Oracle in particular hands back comments as CLOB.
                return STR;
            default:
                return STR;
        }
    }
}
