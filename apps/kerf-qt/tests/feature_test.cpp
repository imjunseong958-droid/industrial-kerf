// Tests for the W3 features on their own: the Sequence Settings dialog. Their
// wiring into the window is pinned by the region tests (toolbar_test:
// sequenceSettingsSendsSetSequence, composerDrawIsTheRegistrysCommand;
// bin_browser_test: titlesAddAGeneratorClipThroughKerfd,
// binAndBrowserKeysAreRegistryCommands; source_test:
// sourcePlayAndRevealAreRegistryCommands).

#include "test_registry.h"

#include "command_registry.h"
#include "sequence_settings.h"
#include "ui/components.h"

#include <QAction>
#include <QComboBox>
#include <QCoreApplication>
#include <QDir>
#include <QJsonArray>
#include <QJsonObject>
#include <QKeySequenceEdit>
#include <QLabel>
#include <QLineEdit>
#include <QScopeGuard>
#include <QSettings>
#include <QSignalSpy>
#include <QSpinBox>
#include <QTest>

namespace kerf {

class FeatureTest final : public QObject {
    Q_OBJECT

    static QJsonObject timeline(bool withClip, const QString &rate = QStringLiteral("25"), double fps = 25, bool drop = false,
                                double start = 0) {
        QJsonArray clips;
        if (withClip) clips.append(QJsonObject{{"id", "c1"}, {"timelineStart", 0.0}, {"duration", 1.0}});
        return QJsonObject{{"sequence", "main"}, {"name", "Atlas"}, {"fps", fps}, {"rate", rate}, {"dropFrame", drop}, {"start", start},
                           {"width", 1920}, {"height", 1080},
                           {"sequences", QJsonArray{QJsonObject{{"id", "main"}, {"name", "Spot"}, {"width", 1920}, {"height", 1080},
                                                                {"rate", rate}, {"fps", fps}, {"dropFrame", drop}}}},
                           {"markers", QJsonArray{}},
                           {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", clips}}}}};
    }

private slots:
    // The dialog reads the program sequence from the snapshot's timeline
    // and writes kerfd's set-sequence with only what changed.
    void sequenceSettingsWritesWhatChanged() {
        const SequenceSettingsDialog::Sequence drop =
            SequenceSettingsDialog::fromTimeline(timeline(false, QStringLiteral("29.97"), 29.97, true, 3600.0));
        QCOMPARE(drop.id, QStringLiteral("main"));
        QCOMPARE(drop.name, QStringLiteral("Spot"));
        QCOMPARE(drop.size, QSize(1920, 1080));
        QVERIFY(drop.empty);
        QCOMPARE(SequenceSettingsDialog::startText(drop), QStringLiteral("01:00:00;00"));
        QCOMPARE(drop.rate, QStringLiteral("29.97"));
        // Both spellings kerfd reads are the same rate.
        QCOMPARE(SequenceSettingsDialog::rateValue(QStringLiteral("30000/1001")), (QJsonObject{{"num", 30000}, {"den", 1001}}));
        QCOMPARE(SequenceSettingsDialog::rateValue(QStringLiteral("29.97")), (QJsonObject{{"num", 30000}, {"den", 1001}}));
        QCOMPARE(SequenceSettingsDialog::rateValue(QStringLiteral("23.976")), (QJsonObject{{"num", 24000}, {"den", 1001}}));
        QCOMPARE(SequenceSettingsDialog::rateValue(QStringLiteral("59.94")), (QJsonObject{{"num", 60000}, {"den", 1001}}));
        QCOMPARE(SequenceSettingsDialog::rateValue(QStringLiteral("12.5")), (QJsonObject{{"num", 25}, {"den", 2}}));
        QVERIFY(SequenceSettingsDialog::sameRate(QStringLiteral("29.97"), QStringLiteral("30000/1001")));
        QVERIFY(!SequenceSettingsDialog::sameRate(QStringLiteral("29.97"), QStringLiteral("30")));
        QCOMPARE(SequenceSettingsDialog::rateValue(QStringLiteral("25")), (QJsonObject{{"num", 25}, {"den", 1}}));
        QVERIFY(SequenceSettingsDialog::rateValue(QStringLiteral("fast")).isEmpty());
        QVERIFY(!SequenceSettingsDialog::fromTimeline(timeline(true)).empty);
        QJsonObject marked = timeline(false);
        marked.insert("markers", QJsonArray{QJsonObject{{"id", "m1"}}});
        QVERIFY(!SequenceSettingsDialog::fromTimeline(marked).empty);

        SequenceSettingsDialog dialog(timeline(false));
        QCOMPARE(dialog.width(), SequenceSettingsDialog::kWidth);
        // Nothing changed: nothing to send, and Apply just closes.
        QVERIFY(dialog.op().isEmpty());
        QSignalSpy sent(&dialog, &SequenceSettingsDialog::applyRequested);
        dialog.show();
        dialog.apply();
        QCOMPARE(sent.size(), 0);
        QCOMPARE(dialog.result(), int(QDialog::Accepted));
        // A custom size: its W × H only then editable.
        QVERIFY(!dialog.widthField()->isEnabled());
        dialog.formatControl()->setCurrentIndex(2, false);
        emit dialog.formatControl()->activated(2);
        QVERIFY(dialog.widthField()->isEnabled());
        dialog.widthField()->setValue(1280);
        dialog.heightField()->setValue(720);
        QCOMPARE(dialog.op(), (QJsonObject{{"op", "set-sequence"}, {"sequence", "main"}, {"width", 1280}, {"height", 720}}));
        // The 9:16 preset puts its size in the fields.
        dialog.formatControl()->setCurrentIndex(1, false);
        emit dialog.formatControl()->activated(1);
        QCOMPARE(dialog.chosenSize(), QSize(1080, 1920));
        QCOMPARE(dialog.widthField()->value(), 1080);
        // A start that is no timecode cannot go.
        dialog.startField()->setText(QStringLiteral("10:00"));
        QString problem;
        QVERIFY(dialog.op(&problem).isEmpty());
        QVERIFY(!problem.isEmpty());
        QVERIFY(!dialog.applyButton()->isEnabled());
        dialog.startField()->setText(QStringLiteral("10:00:00:00"));
        QCOMPARE(dialog.op().value("start").toString(), QStringLiteral("10:00:00:00"));
        // Nor a sequence without a name.
        dialog.nameField()->setText(QStringLiteral("  "));
        QVERIFY(dialog.op(&problem).isEmpty());
        QCOMPARE(problem, QStringLiteral("A sequence needs a name"));
        dialog.nameField()->setText(QStringLiteral("Spot"));
        // An empty sequence takes a new rate.
        dialog.rateField()->setCurrentIndex(dialog.rateField()->findData(QStringLiteral("23.976")));
        QCOMPARE(dialog.op().value("rate").toObject(), (QJsonObject{{"num", 24000}, {"den", 1001}}));
        dialog.apply();
        QCOMPARE(sent.size(), 1);
        // A clip arrives while it is open: the rate goes back to the
        // sequence's, read-only, with the note; typed fields stay.
        dialog.setTimeline(timeline(true));
        QVERIFY(!dialog.rateEditable());
        QVERIFY(!dialog.rateField()->isEnabled());
        QCOMPARE(dialog.rateField()->currentData().toString(), QStringLiteral("25"));
        QVERIFY(dialog.rateNoteLabel()->isVisibleTo(&dialog));
        QVERIFY(!dialog.op().contains("rate"));
        QCOMPARE(dialog.op().value("start").toString(), QStringLiteral("10:00:00:00"));
        // kerfd's refusal shows in neg.
        dialog.setError(SequenceSettingsDialog::rateNote());
        QCOMPARE(dialog.error(), QStringLiteral("Frame rate can change only while the sequence is empty"));
        QVERIFY(dialog.m_error->isVisibleTo(&dialog));
        // The offered rates are those kerfd reads, and the sequence's own
        // is there even when it is none of them.
        QStringList rates;
        for (const auto &rate : SequenceSettingsDialog::rates()) rates << rate.second;
        QVERIFY(rates.contains(QStringLiteral("29.97")) && rates.contains(QStringLiteral("60")));
        SequenceSettingsDialog odd(timeline(false, QStringLiteral("48"), 48));
        QCOMPARE(odd.rateField()->currentData().toString(), QStringLiteral("48"));
        QCOMPARE(odd.rateField()->count(), int(SequenceSettingsDialog::rates().size()) + 1);
    }

    // kerfd writes NTSC rates as decimals ("29.97", Rate's Display): the
    // dialog finds the offered row by value, adds no second one, and sends
    // nothing until a different rate is picked.
    void sequenceSettingsReadsKerfdsRateSpelling() {
        for (const auto &[rate, fps] : {std::pair{QStringLiteral("29.97"), 29.97}, std::pair{QStringLiteral("23.976"), 23.976},
                                        std::pair{QStringLiteral("59.94"), 59.94}, std::pair{QStringLiteral("30000/1001"), 29.97}}) {
            SequenceSettingsDialog dialog(timeline(false, rate, fps, false));
            QCOMPARE(dialog.rateField()->count(), int(SequenceSettingsDialog::rates().size()));
            QVERIFY(SequenceSettingsDialog::sameRate(dialog.rateField()->currentData().toString(), rate));
            QVERIFY(dialog.op().isEmpty());
        }

        SequenceSettingsDialog dialog(timeline(false, QStringLiteral("29.97"), 29.97, false));
        QComboBox *field = dialog.rateField();
        int rows = 0;
        for (int i = 0; i < field->count(); ++i) rows += field->itemText(i) == QStringLiteral("29.97p");
        QCOMPARE(rows, 1);
        QCOMPARE(field->currentText(), QStringLiteral("29.97p"));
        // Picking the same row again is no change.
        field->setCurrentIndex(field->findData(QStringLiteral("29.97")));
        QVERIFY(dialog.op().isEmpty());
        // A different rate goes, as a fraction kerfd reads.
        field->setCurrentIndex(field->findData(QStringLiteral("30")));
        QCOMPARE(dialog.op(), (QJsonObject{{"op", "set-sequence"}, {"sequence", "main"}, {"rate", QJsonObject{{"num", 30}, {"den", 1}}}}));

        // The rate moves under the open dialog: the untouched field follows
        // it by value, and still no row is doubled.
        field->setCurrentIndex(field->findData(QStringLiteral("29.97")));
        dialog.setTimeline(timeline(false, QStringLiteral("59.94"), 59.94, false));
        QCOMPARE(field->currentData().toString(), QStringLiteral("59.94"));
        QCOMPARE(field->count(), int(SequenceSettingsDialog::rates().size()));
        QVERIFY(dialog.op().isEmpty());
        // A rate of the sequence's own that is none of the offered ones gets
        // its row, and picking it back after it moves sends a real rate.
        dialog.setTimeline(timeline(false, QStringLiteral("47.952"), 47.952, false));
        QCOMPARE(field->count(), int(SequenceSettingsDialog::rates().size()) + 1);
        const int own = field->currentIndex();
        dialog.setTimeline(timeline(false, QStringLiteral("25"), 25, false));
        QCOMPARE(field->currentData().toString(), QStringLiteral("25"));
        field->setCurrentIndex(own);
        QCOMPARE(dialog.op().value("rate").toObject(), (QJsonObject{{"num", 48000}, {"den", 1001}}));
    }

    // KERF_SCREENSHOT_DIR=<dir>: the Sequence Settings dialog (empty and
    // with content), saved for a look.
    void featureGallery() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        if (shots.isEmpty()) QSKIP("Set KERF_SCREENSHOT_DIR to save the feature dialogs");
        const auto save = [&shots](QWidget &widget, const QString &name) {
            widget.show();
            QVERIFY(QTest::qWaitForWindowExposed(&widget));
            QCoreApplication::processEvents();
            QVERIFY(widget.grab().save(QDir(shots).filePath(name + QStringLiteral(".png"))));
        };
        SequenceSettingsDialog empty(timeline(false));
        save(empty, QStringLiteral("sequence-settings-empty"));
        SequenceSettingsDialog full(timeline(true));
        full.setError(SequenceSettingsDialog::rateNote());
        save(full, QStringLiteral("sequence-settings-content"));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::FeatureTest)

#include "feature_test.moc"
