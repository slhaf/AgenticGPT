package work.slhaf.agentic.console.platform.attention.persistence

import android.content.Context
import androidx.room.Database
import androidx.room.Room
import androidx.room.RoomDatabase

@Database(
    entities = [AttentionEntity::class],
    version = 1,
    exportSchema = false,
)
abstract class AttentionDatabase : RoomDatabase() {
    abstract fun attentionDao(): AttentionDao

    companion object {
        private const val DATABASE_NAME = "agentic_attention.db"

        @Volatile
        private var instance: AttentionDatabase? = null

        fun create(context: Context): AttentionDatabase =
            instance ?: synchronized(this) {
                instance ?: Room.databaseBuilder(
                    context.applicationContext,
                    AttentionDatabase::class.java,
                    DATABASE_NAME,
                ).build().also { instance = it }
            }
    }
}
